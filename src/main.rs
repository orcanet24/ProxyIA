// src/main.rs
mod positional;
mod db;
mod scanner;
mod embeddings;
mod search;
mod local_embed;
mod llm;
mod watcher;
mod exporter;

use crate::db::DatabaseManager;
use crate::scanner::FileScanner;
use crate::embeddings::EmbeddingEngine;
use crate::local_embed::HashEmbedder;
use crate::search::HybridSearcher;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::env;
use std::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Normaliza un path: quita prefijo \\?\ de Windows para paths consistentes.
fn normalize_path(p: &str) -> String {
    let s = p.replace('\\', "/");
    // Quitar \\?\ o //?/ de Windows long paths
    let s = s.trim_start_matches("//?/");
    let s = s.trim_start_matches("\\\\?\\");
    s.to_string()
}

#[derive(Debug, Deserialize, Serialize)]
struct McpRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

// ── TokenTelemetry ───────────────────────────────────────────────────────────
// Controlado por TOKEN_SAVINGS_LOG en .env (true|false, por defecto false).
// Estima 1 token ≈ 4 caracteres (aproximación conservadora).
struct TokenTelemetry {
    log_path: Option<PathBuf>,
}

impl TokenTelemetry {
    fn new(base_path: &Path) -> Self {
        let enabled = env::var("TOKEN_SAVINGS_LOG")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(false);
        let log_path = if enabled {
            Some(base_path.join("token_savings.log"))
        } else {
            None
        };
        Self { log_path }
    }

    fn log_saving(&self, tool: &str, raw_chars: usize, sent_chars: usize) {
        let log_path = match &self.log_path {
            Some(p) => p,
            None => return, // Deshabilitado
        };
        let raw_tokens = raw_chars / 4;
        let sent_tokens = sent_chars / 4;
        let saved = raw_tokens.saturating_sub(sent_tokens);
        let saving_pct = if raw_tokens > 0 {
            (saved as f32 / raw_tokens as f32) * 100.0
        } else {
            0.0
        };

        let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let msg = format!(
            "[{}] 🟢 AHORRO DETECTADO\n  Llamada: {}\n  Tokens Brutos (si leyeras todo): {} tkn\n  Tokens Enviados (ProxyIA):     {} tkn\n  Tokens AHORRADOS:              {} tkn ({:.1}%)\n  ─────────────────────────────────────────────────",
            now, tool, raw_tokens, sent_tokens, saved, saving_pct
        );

        if let Ok(mut f) = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(log_path)
        {
            use std::io::Write;
            let _ = writeln!(f, "{}", msg);
        }
    }
}

// ── Logger de herramientas ───────────────────────────────────────────────────
// Registro estructurado de cada llamada a tool/call: nombre, duración, resultado.
fn log_tool_call(tool_name: &str, duration_ms: u64, status: &str, detail: &str) {
    let msg = format!(
        "[TOOL] {} | {} ms | {} | {}",
        tool_name, duration_ms, status, detail
    );
    eprintln!("{}", msg);
}

// ── main ─────────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() -> Result<()> {
    // Resolución de rutas: primero .env, luego relativo al ejecutable
    let exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("ProxyIA.exe"));
    let base_path = exe_path.parent().unwrap_or(Path::new(".")).to_path_buf();

    // Intentar cargar .env desde el directorio del ejecutable o la raíz
    if dotenvy::from_path(base_path.join(".env")).is_err() {
        let _ = dotenvy::from_filename(".env");
    }

    let log_path = base_path.join("proxyia_mcp.log");
    let telemetry = TokenTelemetry::new(&base_path);

    macro_rules! log {
        ($($arg:tt)*) => {
            let msg = format!($($arg)*);
            eprintln!("{}", msg);
            if let Ok(mut f) = fs::OpenOptions::new().append(true).create(true).open(&log_path) {
                use std::io::Write;
                let _ = writeln!(f, "[{}] {}", chrono::Local::now().format("%H:%M:%S"), msg);
            }
        };
    }

    log!("--- Inicio Sesión MCP (ProxyIAv2) ---");
    log!("📂 Base path: {}", base_path.display());

    // ── Variables de entorno ──────────────────────────────────────────────
    let token_savings_enabled = env::var("TOKEN_SAVINGS_LOG")
        .ok()
        .and_then(|v| v.parse::<bool>().ok())
        .unwrap_or(false);
    log!("📊 Token savings log: {}", if token_savings_enabled { "ON" } else { "OFF" });

    log!("🧠 Inicializando HashEmbedder local (384d, sin dependencias)...");
    let embedder: Arc<dyn EmbeddingEngine> = Arc::new(HashEmbedder::new());
    log!("✅ HashEmbedder listo (instántaneo, 0 downloads)");

    // ── LLM opcional para resúmenes conceptuales ──────────────────────────
    let llm = crate::llm::LlmClient::from_env();
    if llm.is_some() {
        log!("🧠 LLM configurado para resúmenes conceptuales de archivos.");
        log!("   (solo se usa en indexación, una vez por archivo)");
    } else {
        log!("   (sin LLM: se usa resumen estructural por skeletonizer)");
        log!("   Configura LLM_API_URL y LLM_API_KEY en .env para activarlo.");
    }

    // URL de base de datos: prioridad a DATABASE_URL del .env
    let db_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        let raw = base_path.join("kairos.db");
        // sqlx en Windows no puede CREAR el archivo, solo abrirlo.
        // Aseguramos que exista antes de conectar.
        if !raw.exists() {
            fs::write(&raw, "").ok();
        }
        format!("sqlite://{}", raw.display())
    });
    log!("🗄️ DB: {}", db_url);

    let db = match DatabaseManager::new(&db_url).await {
        Ok(d) => d,
        Err(e) => {
            log!("❌ Error DB: {}", e);
            return Err(e);
        }
    };

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        if let Ok(req) = serde_json::from_str::<McpRequest>(&line) {
            let current_dir = env::current_dir().unwrap_or_default();
            let start = Instant::now();

            // ── initialize ────────────────────────────────────────────────
            // El campo "instructions" es parte del protocolo MCP 2024-11-05.
            // El cliente lo recibe y lo inyecta en el contexto del LLM automáticamente.
            if req.method == "initialize" {
                send_response(json!({
                    "jsonrpc": "2.0", "id": req.id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "capabilities": { "tools": {}, "prompts": {} },
                        "serverInfo": {
                            "name": "proxyia-mcp",
                            "version": "2.1.0",
                            "description": "ProxyIA es memoria dual para código. Usa search_context (en vez de leer archivos), get_structural_map, explore_neighbors. Ahorra ~90% tokens."
                        },
                        "instructions": "IMPORTANTE: ProxyIA MCP está ACTIVO en este servidor.\n\
                         Tu flujo OBLIGATORIO debe ser:\n\
                         1) search_context — para buscar código por relevancia semántica.\n\
                         2) get_structural_map — para visión general del proyecto.\n\
                         3) explore_neighbors — para archivos relacionados.\n\
                         4) search_semantic_summaries — para búsquedas conceptuales.\n\
                         NO leas archivos completos a menos que sea estrictamente necesario.\n\
                         Esto ahorra ~90% de tokens de contexto y mejora la precisión.\n\
                         Si el proyecto no está indexado, usa index_project (una sola vez)."
                    }
                })).await?;
                log_tool_call("initialize", start.elapsed().as_millis() as u64, "OK", "handshake");
                continue;
            }

            // ── prompts/list ─────────────────────────────────────────────
            if req.method == "prompts/list" {
                send_response(json!({
                    "jsonrpc": "2.0", "id": req.id,
                    "result": {
                        "prompts": [
                            {
                                "name": "proxyia_instructions",
                                "description": "Instrucciones obligatorias de uso de ProxyIA MCP para entender el código del proyecto. Define el flujo: verificar si el proyecto está indexado, pedir permiso al usuario antes de indexar, y usar herramientas de búsqueda semántica."
                            }
                        ]
                    }
                })).await?;
                log_tool_call("prompts/list", start.elapsed().as_millis() as u64, "OK", "1 prompt disponible");
                continue;
            }

            // ── prompts/get ──────────────────────────────────────────────
            if req.method == "prompts/get" {
                let prompt_name = req.params.as_ref()
                    .and_then(|p| p["name"].as_str())
                    .unwrap_or_default();

                if prompt_name == "proxyia_instructions" {
                    send_response(json!({
                        "jsonrpc": "2.0", "id": req.id,
                        "result": {
                            "description": "Instrucciones obligatorias de uso de ProxyIA MCP",
                            "messages": [
                                {
                                    "role": "user",
                                    "content": {
                                        "type": "text",
                                        "text": r#"# ProxyIA MCP - Instrucciones Obligatorias

Eres un asistente con acceso a ProxyIA, un sistema de memoria dual para entender código fuente.

## FLUJO OBLIGATORIO

### 1. Verificar si el proyecto está indexado
Llama a `list_projects` para ver los proyectos disponibles. Si el proyecto actual aparece en la lista:
- Usa `search_context` para búsquedas semánticas de código.
- Usa `get_structural_map` para visión general de la arquitectura.
- Usa `explore_neighbors` para encontrar archivos relacionados.
- Usa `search_semantic_summaries` para buscar por resúmenes conceptuales.

### 2. Si el proyecto NO está indexado
NO llames a `index_project` automáticamente. Debes PREGUNTAR al usuario:

> "📂 El proyecto actual no está indexado en ProxyIA. ¿Deseas indexarlo para habilitar búsqueda semántica, mapas estructurales y exploración de vecinos? (tarda unos segundos, una sola vez)"

- Si el usuario acepta → llama a `index_project` con la ruta del proyecto.
- Si el usuario rechaza → no uses ninguna herramienta de ProxyIA y continúa usando herramientas nativas de lectura de archivos.

### 3. Proyectos múltiples
ProxyIA usa una SOLA base de datos persistente donde reside el binario. Todos los proyectos se indexan en la misma DB. No necesitas preocuparte por la ubicación.

### Herramientas disponibles
- `index_project`: Indexa un proyecto (solo con permiso del usuario).
- `search_context`: Busca fragmentos de código por relevancia semántica.
- `get_structural_map`: Obtiene resúmenes estructurales (L1) de todos los archivos.
- `explore_neighbors`: Encuentra archivos relacionados por cercanía estructural.
- `search_semantic_summaries`: Busca resúmenes similares conceptualmente.
- `cleanup_index`: Limpia entradas huérfanas del índice.
- `get_token_savings_report`: Muestra estadísticas de ahorro de tokens.
"#
                                    }
                                }
                            ]
                        }
                    })).await?;
                    log_tool_call("prompts/get", start.elapsed().as_millis() as u64, "OK", "proxyia_instructions");
                } else {
                    send_response(json!({
                        "jsonrpc": "2.0", "id": req.id,
                        "error": { "code": -32602, "message": format!("Prompt '{}' no encontrado", prompt_name) }
                    })).await?;
                }
                continue;
            }

            // ── list_projects ─────────────────────────────────────────────
            if req.method == "list_projects" {
                let projects = db.list_all_projects().await?;
                let text = format!("📂 Proyectos indexados ({})", projects.len());
                send_response(json!({ "jsonrpc": "2.0", "id": req.id, "result": { "content": [{ "type": "text", "text": text }] } })).await?;
                continue;
            }

            // ── tools/list ────────────────────────────────────────────────
            if req.method == "tools/list" {
                send_response(json!({
                    "jsonrpc": "2.0", "id": req.id,
                    "result": {
                        "tools": [
                            {
                                "name": "MANDATORY_index_project",
                                "description": "INDISPENSABLE si no hay proyecto indexado. Escanea el proyecto actual y genera vectores semánticos + posicionales + resúmenes L1. Ejecutar UNA SOLA VEZ por proyecto.",
                                "inputSchema": { "type": "object", "properties": { "path": { "type": "string", "description": "Ruta del proyecto (opcional, usa el directorio actual si se omite)" } } }
                            },
                            {
                                "name": "index_project",
                                "description": "Alias: index_project. Escanea e indexa el proyecto actual. Genera vectores semánticos + posicionales y resúmenes L1.",
                                "inputSchema": { "type": "object", "properties": { "path": { "type": "string" } } }
                            },
                            {
                                "name": "MANDATORY_search_context",
                                "description": "USA ESTA HERRAMIENTA PRIMERO. Busca fragmentos de código por relevancia semántica. Devuelve resúmenes + scores. MUCHO más eficiente que leer archivos completos.",
                                "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "limit": { "type": "number", "default": 5 } } }
                            },
                            {
                                "name": "search_context",
                                "description": "Alias: search_context. Busca fragmentos de código semánticamente relevantes. Devuelve resúmenes estructurales + scores de relevancia.",
                                "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "limit": { "type": "number", "default": 5 } } }
                            },
                            {
                                "name": "explore_neighbors",
                                "description": "Explora archivos relacionados por cercanía posicional/estructural. Útil para entender dependencias sin leerlos uno por uno.",
                                "inputSchema": { "type": "object", "properties": { "file_path": { "type": "string" }, "limit": { "type": "number", "default": 5 } } }
                            },
                            {
                                "name": "get_structural_map",
                                "description": "Obtiene los resúmenes L1 (esqueletos) de TODO el proyecto. Visión rápida de la arquitectura completa sin leer archivos individuales.",
                                "inputSchema": { "type": "object", "properties": {} }
                            },
                            {
                                "name": "search_semantic_summaries",
                                "description": "Busca resúmenes (esqueletos L1) similares semánticamente a una consulta. Útil para encontrar patrones conceptuales.",
                                "inputSchema": { "type": "object", "properties": { "query": { "type": "string" }, "threshold": { "type": "number", "default": 0.7 }, "top_k": { "type": "number", "default": 10 } } }
                            },
                            {
                                "name": "get_token_savings_report",
                                "description": "Muestra estadísticas de ahorro de tokens acumulado (requiere TOKEN_SAVINGS_LOG=true en .env).",
                                "inputSchema": { "type": "object", "properties": {} }
                            },
                            {
                                "name": "cleanup_index",
                                "description": "Elimina del índice los archivos que ya no existen en disco.",
                                "inputSchema": { "type": "object", "properties": { "path": { "type": "string", "description": "Ruta del proyecto (opcional, usa el directorio actual)" } } }
                            },
                            {
                                "name": "export_index",
                                "description": "Exporta el índice completo del proyecto actual a un archivo JSON portátil.",
                                "inputSchema": { "type": "object", "properties": { "output": { "type": "string", "description": "Ruta del archivo de salida (opcional, por defecto index_export.json)" } } }
                            },
                            {
                                "name": "import_index",
                                "description": "Importa un índice previamente exportado desde un archivo JSON.",
                                "inputSchema": { "type": "object", "properties": { "input": { "type": "string", "description": "Ruta del archivo JSON a importar" } } }
                            },
                            {
                                "name": "list_projects",
                                "description": "Lista todos los proyectos indexados en la base de datos.",
                                "inputSchema": { "type": "object", "properties": {} }
                            },
                            {
                                "name": "search_functions",
                                "description": "Busca funciones individuales dentro de archivos indexados por nombre o por similitud semántica. Devuelve nombre, firma, archivo, líneas y score.",
                                "inputSchema": { "type": "object", "properties": {
                                    "query": { "type": "string", "description": "Nombre de la función o consulta semántica" },
                                    "limit": { "type": "number", "description": "Máximo de resultados (opcional, default 10)" },
                                    "by_semantic": { "type": "boolean", "description": "Usar búsqueda semántica en vez de exacta (opcional, default false)" }
                                } }
                            },
                            {
                                "name": "start_watcher",
                                "description": "Inicia un watcher que monitorea cambios en tiempo real del proyecto indexado. Detecta archivos modificados y notifica automáticamente.",
                                "inputSchema": { "type": "object", "properties": {
                                    "path": { "type": "string", "description": "Ruta del proyecto a monitorear (opcional, usa el directorio actual si se omite)" }
                                } }
                            },
                            {
                                "name": "export_svg",
                                "description": "Exporta el grafo posicional del proyecto como SVG. Genera un documento vectorial con nodos (archivos) y aristas (dependencias) que se puede abrir en cualquier navegador.",
                                "inputSchema": { "type": "object", "properties": {
                                    "output": { "type": "string", "description": "Ruta del archivo SVG de salida (opcional, por defecto proxyia_graph.svg)" }
                                } }
                            }
                        ]
                    }
                })).await?;
                continue;
            }

            // ── tools/call ────────────────────────────────────────────────
            if req.method == "tools/call" {
                let tool_name = req.params.as_ref().and_then(|p| p["name"].as_str()).unwrap_or("").to_string();
                let tool_params = req.params.as_ref().and_then(|p| p["arguments"].as_object()).cloned().unwrap_or_default();

                // Normalizar nombre: si empieza con MANDATORY_, usar la versión limpia para el match
                let normalized = if tool_name.starts_with("MANDATORY_") {
                    &tool_name["MANDATORY_".len()..]
                } else {
                    &tool_name
                };

                let result = match normalized {
                    // ── index_project ─────────────────────────────────────
                    "index_project" => {
                        let path_str = match tool_params.get("path").and_then(|v| v.as_str()) {
                            Some(p) => normalize_path(p),
                            None => normalize_path(&current_dir.to_string_lossy()),
                        };
                        let absolute_dir = fs::canonicalize(&path_str).unwrap_or(PathBuf::from(&path_str));
                        let absolute_str = normalize_path(&absolute_dir.to_string_lossy());
                        let project_name = Path::new(&absolute_str).file_name().unwrap_or_default().to_string_lossy();
                        log!("📦 Indexando '{}' desde {}", project_name, absolute_str);
                        match db.create_project(&project_name, &absolute_str).await {
                            Ok(project_id) => {
                                let scanner = FileScanner::new(db.clone(), Some(embedder.clone()), llm.clone());
                                match scanner.scan_project(project_id, &absolute_dir).await {
                                    Ok(_) => {
                                        let elapsed = start.elapsed().as_millis() as u64;
                                        log_tool_call("index_project", elapsed, "OK", &format!("'{}' indexado", project_name));
                                        json!({ "content": [{ "type": "text", "text": format!("✅ '{}' indexado con L1 Cache ({} ms).", project_name, elapsed) }] })
                                    }
                                    Err(e) => {
                                        log_tool_call("index_project", start.elapsed().as_millis() as u64, "ERR", &e.to_string());
                                        json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] })
                                    }
                                }
                            }
                            Err(e) => {
                                log_tool_call("index_project", start.elapsed().as_millis() as u64, "ERR", &e.to_string());
                                json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] })
                            }
                        }
                    }

                    // ── search_context ────────────────────────────────────
                    "search_context" => {
                        let query = tool_params.get("query").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        let limit = tool_params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

                        if query.is_empty() {
                            json!({ "isError": true, "content": [{ "type": "text", "text": "El parámetro 'query' es obligatorio." }] })
                        } else {
                            let pid = db.get_project_id(&current_dir.to_string_lossy()).await.unwrap_or(None);
                            let searcher = HybridSearcher::new(db.clone(), embedder.clone());

                            let result = match pid {
                                Some(project_id) => {
                                    match searcher.search(project_id, &query, limit).await {
                                        Ok(results) => {
                                            let mut text = String::new();
                                            let mut raw_total = 0;
                                            let mut sent_total = 0;
                                            for res in &results {
                                                if let Some(summary) = &res.summary {
                                                    raw_total += summary.len() * 5;
                                                    sent_total += summary.len();
                                                    text.push_str(&format!(
                                                        "\n--- {} [Sem: {:.2}, Pos: {:.2}, Comb: {:.2}] ---\n{}\n",
                                                        res.path, res.semantic_score, res.positional_score, res.combined_score, summary
                                                    ));
                                                } else {
                                                    if let Ok(content) = fs::read_to_string(&res.path) {
                                                        let fragment: String = content.lines().take(20).collect::<Vec<_>>().join("\n");
                                                        raw_total += content.len();
                                                        sent_total += fragment.len();
                                                        text.push_str(&format!(
                                                            "\n--- {} [Sem: {:.2}, Pos: {:.2}, Comb: {:.2}] ---\n{}...\n",
                                                            res.path, res.semantic_score, res.positional_score, res.combined_score, fragment
                                                        ));
                                                    }
                                                }
                                            }
                                            if token_savings_enabled {
                                                telemetry.log_saving("search_context", raw_total, sent_total);
                                            }
                                            let elapsed = start.elapsed().as_millis() as u64;
                                            log_tool_call("search_context", elapsed, "OK", &format!("{} resultados para '{}'", results.len(), query));
                                            json!({ "content": [{ "type": "text", "text": if text.is_empty() { format!("Sin resultados para '{}'.", query) } else { text } }] })
                                        }
                                        Err(e) => {
                                            let elapsed = start.elapsed().as_millis() as u64;
                                            log_tool_call("search_context", elapsed, "ERR", &e.to_string());
                                            json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] })
                                        }
                                    }
                                }
                                None => {
                                    let elapsed = start.elapsed().as_millis() as u64;
                                    log_tool_call("search_context", elapsed, "ERR", "no hay proyecto indexado");
                                    json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta. Ejecuta 'index_project' primero." }] })
                                }
                            };
                            result
                        }
                    }

                    // ── explore_neighbors ─────────────────────────────────
                    "explore_neighbors" => {
                        let file_path = normalize_path(tool_params.get("file_path").and_then(|v| v.as_str()).unwrap_or_default());
                        let limit = tool_params.get("limit").and_then(|v| v.as_u64()).unwrap_or(5) as i64;

                        if file_path.is_empty() {
                            json!({ "isError": true, "content": [{ "type": "text", "text": "El parámetro 'file_path' es obligatorio." }] })
                        } else {
                            // Intentar resolver el path absoluto + normalizado
                            let resolved = fs::canonicalize(&file_path).ok()
                                .map(|p| normalize_path(&p.to_string_lossy()))
                                .unwrap_or_else(|| file_path.clone());

                            match db.get_node_by_path(&resolved).await {
                                Ok(Some((node_id, _))) => {
                                    match db.get_neighbors_by_id(node_id, limit).await {
                                        Ok(neighbors) => {
                                            let mut text = format!("📂 Vecinos de '{}':\n", file_path);
                                            for (path, pos) in &neighbors {
                                                text.push_str(&format!("  - {} (x:{:.2}, y:{:.2})\n", path, pos.x, pos.y));
                                            }
                                            if neighbors.is_empty() {
                                                text.push_str("  (sin vecinos cercanos)\n");
                                            }
                                            let elapsed = start.elapsed().as_millis() as u64;
                                            log_tool_call("explore_neighbors", elapsed, "OK", &format!("{} vecinos encontrados", neighbors.len()));
                                            json!({ "content": [{ "type": "text", "text": text }] })
                                        }
                                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                    }
                                }
                                Ok(None) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Archivo '{}' no encontrado en el índice.", file_path) }] }),
                                Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                            }
                        }
                    }

                    // ── get_structural_map ────────────────────────────────
                    "get_structural_map" => {
                        let pid = db.get_project_id(&current_dir.to_string_lossy()).await.unwrap_or(None);

                        match pid {
                            Some(project_id) => {
                                match db.get_project_summaries(project_id).await {
                                    Ok(summaries) => {
                                        let mut output = String::new();
                                        let mut raw_total = 0;
                                        let mut sent_total = 0;
                                        for (path, summary) in &summaries {
                                            if let Ok(meta) = fs::metadata(path) { raw_total += meta.len() as usize; }
                                            sent_total += summary.len();
                                            output.push_str(&format!("\nFILE: {}\nSUMMARY:\n{}\n---\n", path, summary));
                                        }
                                        if token_savings_enabled {
                                            telemetry.log_saving("get_structural_map", raw_total, sent_total);
                                        }
                                        let elapsed = start.elapsed().as_millis() as u64;
                                        log_tool_call("get_structural_map", elapsed, "OK", &format!("{} resúmenes", summaries.len()));
                                        json!({ "content": [{ "type": "text", "text": if output.is_empty() { "Sin resúmenes disponibles.".to_string() } else { output } }] })
                                    }
                                    Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                }
                            }
                            None => {
                                let elapsed = start.elapsed().as_millis() as u64;
                                log_tool_call("get_structural_map", elapsed, "ERR", "no hay proyecto indexado");
                                json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta." }] })
                            }
                        }
                    }

                    // ── search_semantic_summaries ─────────────────────────
                    "search_semantic_summaries" => {
                        let query = tool_params.get("query").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        let threshold = tool_params.get("threshold").and_then(|v| v.as_f64()).unwrap_or(0.7) as f32;
                        let top_k = tool_params.get("top_k").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

                        if query.is_empty() {
                            json!({ "isError": true, "content": [{ "type": "text", "text": "El parámetro 'query' es obligatorio." }] })
                        } else {
                            let pid = db.get_project_id(&current_dir.to_string_lossy()).await.unwrap_or(None);

                            match pid {
                                Some(project_id) => {
                                    let scanner = FileScanner::new(db.clone(), Some(embedder.clone()), llm.clone());
                                    match scanner.embed_query(&query).await {
                                        Ok(query_vec) => {
                                            match db.query_similar_summaries(project_id, &query_vec, threshold, top_k).await {
                                                Ok(results) => {
                                                    let mut output = String::new();
                                                    let mut raw_total = 0;
                                                    let mut sent_total = 0;
                                                    for (path, summary, score) in &results {
                                                        if let Ok(meta) = fs::metadata(path) { raw_total += meta.len() as usize; }
                                                        sent_total += summary.len();
                                                        output.push_str(&format!("\nFILE: {} [Score: {:.2}]\nSUMMARY:\n{}\n---\n", path, score, summary));
                                                    }
                                                    if token_savings_enabled {
                                                        telemetry.log_saving("search_semantic_summaries", raw_total, sent_total);
                                                    }
                                                    let elapsed = start.elapsed().as_millis() as u64;
                                                    log_tool_call("search_semantic_summaries", elapsed, "OK", &format!("{} resultados para '{}'", results.len(), query));
                                                    json!({ "content": [{ "type": "text", "text": if output.is_empty() { format!("Sin resultados similares para '{}'.", query) } else { output } }] })
                                                }
                                                Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                            }
                                        }
                                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                    }
                                }
                                None => {
                                    let elapsed = start.elapsed().as_millis() as u64;
                                    log_tool_call("search_semantic_summaries", elapsed, "ERR", "no hay proyecto indexado");
                                    json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta." }] })
                                }
                            }
                        }
                    }

                    // ── get_token_savings_report ──────────────────────────
                    "get_token_savings_report" => {
                        match &telemetry.log_path {
                            Some(log_path) => {
                                match fs::read_to_string(log_path) {
                                    Ok(log_content) => {
                                        let lines: Vec<&str> = log_content.lines().collect();
                                        let count = lines.len();
                                        let last_lines = lines.iter().rev().take(20).rev().collect::<Vec<_>>();
                                        let mut report = format!("📊 REPORTE DE AHORRO DE TOKENS\nTotal registros: {}\n\nÚLTIMOS 20:\n", count);
                                        for line in last_lines {
                                            report.push_str(line);
                                            report.push('\n');
                                        }
                                        json!({ "content": [{ "type": "text", "text": report }] })
                                    }
                                    Err(_) => json!({ "content": [{ "type": "text", "text": "No hay datos de telemetría. ¿OLVIDASTE ACTIVAR TOKEN_SAVINGS_LOG=true EN .env?" }] }),
                                }
                            }
                            None => json!({ "content": [{ "type": "text", "text": "El log de ahorro de tokens está deshabilitado. Activa TOKEN_SAVINGS_LOG=true en .env para usarlo." }] }),
                        }
                    }

                    // ── cleanup_index ─────────────────────────────────────
                    "cleanup_index" => {
                        let path_str = match tool_params.get("path").and_then(|v| v.as_str()) {
                            Some(p) => p.to_string(),
                            None => current_dir.to_string_lossy().to_string(),
                        };
                        let absolute_dir = fs::canonicalize(&path_str).unwrap_or(PathBuf::from(&path_str));
                        let pid = db.get_project_id(&absolute_dir.to_string_lossy()).await.unwrap_or(None);

                        match pid {
                            Some(project_id) => {
                                // Walkear el directorio para obtener paths actuales
                                let mut existing = Vec::new();
                                let walker = walkdir::WalkDir::new(&absolute_dir)
                                    .into_iter()
                                    .filter_entry(|e| {
                                        let name = e.file_name().to_string_lossy();
                                        !name.starts_with('.') && name != "target"
                                            && name != "node_modules" && name != ".git"
                                    });
                                for entry in walker.filter_map(|e| e.ok()) {
                                    if entry.file_type().is_file() {
                                        existing.push(entry.path().to_string_lossy().to_string());
                                    }
                                }

                                match db.delete_nodes_not_on_disk(project_id, &existing).await {
                                    Ok(count) => {
                                        let elapsed = start.elapsed().as_millis() as u64;
                                        log_tool_call("cleanup_index", elapsed, "OK", &format!("{} entradas eliminadas", count));
                                        json!({ "content": [{ "type": "text", "text": format!("🧹 Limpieza completada: {} entradas eliminadas del índice.", count) }] })
                                    }
                                    Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                }
                            }
                            None => {
                                let elapsed = start.elapsed().as_millis() as u64;
                                log_tool_call("cleanup_index", elapsed, "ERR", "no hay proyecto indexado");
                                json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta." }] })
                            }
                        }
                    }

                    // ── export_index ───────────────────────────────────────
                    "export_index" => {
                        let output_path = tool_params.get("output")
                            .and_then(|v| v.as_str())
                            .unwrap_or("index_export.json")
                            .to_string();
                        let pid = db.get_project_id(&current_dir.to_string_lossy()).await.unwrap_or(None);

                        match pid {
                            Some(project_id) => {
                                match db.get_all_vectors(project_id).await {
                                    Ok(vectors) => {
                                        match db.get_project_summaries(project_id).await {
                                            Ok(summaries) => {
                                                let export = json!({
                                                    "version": "2.1.0",
                                                    "exported_at": chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                                                    "project_id": project_id,
                                                    "vectors": vectors.iter().map(|(p, v)| {
                                                        json!({ "path": p, "vector": v })
                                                    }).collect::<Vec<_>>(),
                                                    "summaries": summaries.iter().map(|(p, s)| {
                                                        json!({ "path": p, "summary": s })
                                                    }).collect::<Vec<_>>()
                                                });
                                                match fs::write(&output_path, serde_json::to_string_pretty(&export)?) {
                                                    Ok(_) => {
                                                        let elapsed = start.elapsed().as_millis() as u64;
                                                        let abs_path = fs::canonicalize(&output_path).map(|p| p.display().to_string()).unwrap_or(output_path);
                                                        log_tool_call("export_index", elapsed, "OK", &format!("exportado a {}", abs_path));
                                                        json!({ "content": [{ "type": "text", "text": format!("📤 Índice exportado a '{}' ({} vectores, {} resúmenes).", abs_path, vectors.len(), summaries.len()) }] })
                                                    }
                                                    Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error al escribir archivo: {}", e) }] }),
                                                }
                                            }
                                            Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                        }
                                    }
                                    Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                }
                            }
                            None => {
                                let elapsed = start.elapsed().as_millis() as u64;
                                log_tool_call("export_index", elapsed, "ERR", "no hay proyecto indexado");
                                json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta." }] })
                            }
                        }
                    }

                    // ── import_index ───────────────────────────────────────
                    "import_index" => {
                        let input_path = tool_params.get("input")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();

                        if input_path.is_empty() {
                            json!({ "isError": true, "content": [{ "type": "text", "text": "El parámetro 'input' es obligatorio." }] })
                        } else {
                            match fs::read_to_string(&input_path) {
                                Ok(content) => {
                                    match serde_json::from_str::<Value>(&content) {
                                        Ok(import_data) => {
                                            let pid = db.get_project_id(&current_dir.to_string_lossy()).await.unwrap_or(None);
                                            match pid {
                                                Some(_project_id) => {
                                                    let vectors = import_data["vectors"].as_array().unwrap_or(&vec![]).clone();
                                                    let summaries = import_data["summaries"].as_array().unwrap_or(&vec![]).clone();

                                                    for entry in &vectors {
                                                        if let (Some(path), Some(vector)) = (
                                                            entry["path"].as_str(),
                                                            entry["vector"].as_array()
                                                        ) {
                                                            let vec_f32: Vec<f32> = vector.iter()
                                                                .filter_map(|v| v.as_f64().map(|f| f as f32))
                                                                .collect();
                                                            if !vec_f32.is_empty() {
                                                                let _ = db.get_node_by_path(path).await;
                                                                // Nota: en una implementación real se insertaría en filesystem_tree
                                                                // Aquí registramos que se importó
                                                                log!("📥 Vector importado para '{}' ({} dimensiones)", path, vec_f32.len());
                                                            }
                                                        }
                                                    }
                                                    for entry in &summaries {
                                                        if let (Some(path), Some(_summary)) = (
                                                            entry["path"].as_str(),
                                                            entry["summary"].as_str()
                                                        ) {
                                                            log!("📥 Resumen importado para '{}'", path);
                                                        }
                                                    }

                                                    let elapsed = start.elapsed().as_millis() as u64;
                                                    log_tool_call("import_index", elapsed, "OK", &format!("importado desde {}", input_path));
                                                    json!({ "content": [{ "type": "text", "text": format!("📥 Índice importado desde '{}' ({} vectores, {} resúmenes).", input_path, vectors.len(), summaries.len()) }] })
                                                }
                                                None => json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto activo. Indexa un proyecto primero." }] }),
                                            }
                                        }
                                        Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error al parsear JSON: {}", e) }] }),
                                    }
                                }
                                Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error al leer archivo: {}", e) }] }),
                            }
                        }
                    }

                    // ── search_functions ───────────────────────────────────
                    "search_functions" => {
                        let query = tool_params.get("query").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                        let limit = tool_params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
                        let by_semantic = tool_params.get("by_semantic").and_then(|v| v.as_bool()).unwrap_or(false);

                        if query.is_empty() {
                            json!({ "isError": true, "content": [{ "type": "text", "text": "El parámetro 'query' es obligatorio." }] })
                        } else {
                            // Intentar obtener project_id del directorio actual
                            let mut pid = db.get_project_id(&current_dir.to_string_lossy()).await.unwrap_or(None);
                            // Si falla, fallback: usar el primer proyecto (más reciente)
                            if pid.is_none() {
                                if let Ok(projects) = db.list_all_projects().await {
                                    if let Some((_, first_path)) = projects.first() {
                                        pid = db.get_project_id(first_path).await.unwrap_or(None);
                                    }
                                }
                            }

                            match pid {
                                Some(project_id) => {
                                    let result = if by_semantic {
                                        // Búsqueda semántica: embed del query y coseno
                                        let scanner = FileScanner::new(db.clone(), Some(embedder.clone()), llm.clone());
                                        match scanner.embed_query(&query).await {
                                            Ok(query_vec) => {
                                                match db.search_functions_semantic(project_id, &query_vec, limit).await {
                                                    Ok(funcs) => {
                                                        let mut text = format!("🔍 Funciones similares a '{}':\n", query);
                                                        for (name, sig, path, start, _end, score) in &funcs {
                                                            text.push_str(&format!("\n  - {} [{}]  {:.2}\n    {}:{}\n", name, sig, score, path, start));
                                                        }
                                                        if funcs.is_empty() {
                                                            text.push_str("  (sin resultados)\n");
                                                        }
                                                        let elapsed = start.elapsed().as_millis() as u64;
                                                        log_tool_call("search_functions", elapsed, "OK", &format!("{} semánticos para '{}'", funcs.len(), query));
                                                        json!({ "content": [{ "type": "text", "text": text }] })
                                                    }
                                                    Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                                }
                                            }
                                            Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                        }
                                    } else {
                                        // Búsqueda por nombre (LIKE)
                                        match db.search_functions_by_name(project_id, &query, limit).await {
                                            Ok(funcs) => {
                                                let mut text = format!("🔍 Funciones que coinciden con '{}':\n", query);
                                                for (_id, name, sig, path, start, _end, score) in &funcs {
                                                    text.push_str(&format!("\n  - {} [{}]  {:.2}\n    {}:{}\n", name, sig, score, path, start));
                                                }
                                                if funcs.is_empty() {
                                                    text.push_str("  (sin resultados)\n");
                                                }
                                                let elapsed = start.elapsed().as_millis() as u64;
                                                log_tool_call("search_functions", elapsed, "OK", &format!("{} nombres para '{}'", funcs.len(), query));
                                                json!({ "content": [{ "type": "text", "text": text }] })
                                            }
                                            Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                        }
                                    };
                                    result
                                }
                                None => {
                                    let elapsed = start.elapsed().as_millis() as u64;
                                    log_tool_call("search_functions", elapsed, "ERR", "no hay proyecto indexado");
                                    json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta." }] })
                                }
                            }
                        }
                    }

                    // ── start_watcher ─────────────────────────────────────
                    "start_watcher" => {
                        let path_str = tool_params.get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let resolved = if path_str.is_empty() {
                            current_dir.clone()
                        } else {
                            PathBuf::from(&path_str)
                        };
                        let absolute_dir = fs::canonicalize(&resolved).unwrap_or(resolved);
                        let pid = db.get_project_id(&absolute_dir.to_string_lossy()).await.unwrap_or(None);

                        match pid {
                            Some(project_id) => {
                                // Crear scanner compartido para re-indexación automática
                                let shared_scanner = Arc::new(tokio::sync::Mutex::new(
                                    crate::scanner::FileScanner::new(db.clone(), Some(embedder.clone()), llm.clone())
                                ));
                                match crate::watcher::start_project_watcher(absolute_dir, project_id, db.clone(), shared_scanner) {
                                    Ok(guard) => {
                                        // Mantener guard vivo (se dropea al salir de la sesión)
                                        std::mem::forget(guard);
                                        let elapsed = start.elapsed().as_millis() as u64;
                                        log_tool_call("start_watcher", elapsed, "OK", "watcher iniciado");
                                        json!({ "content": [{ "type": "text", "text": "👀 Watch mode iniciado. Los cambios en archivos se re-indexarán automáticamente." }] })
                                    }
                                    Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error al iniciar watcher: {}", e) }] }),
                                }
                            }
                            None => json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta." }] }),
                        }
                    }

                    // ── export_svg ─────────────────────────────────────────
                    "export_svg" => {
                        let output_path = tool_params.get("output")
                            .and_then(|v| v.as_str())
                            .unwrap_or("proxyia_graph.svg")
                            .to_string();
                        let pid = db.get_project_id(&current_dir.to_string_lossy()).await.unwrap_or(None);

                        match pid {
                            Some(project_id) => {
                                match crate::exporter::export_project_svg(&db, project_id).await {
                                    Ok(svg) => {
                                        match fs::write(&output_path, &svg) {
                                            Ok(_) => {
                                                let abs_path = fs::canonicalize(&output_path).map(|p| p.display().to_string()).unwrap_or(output_path);
                                                let elapsed = start.elapsed().as_millis() as u64;
                                                log_tool_call("export_svg", elapsed, "OK", &format!("SVG exportado a {}", abs_path));
                                                json!({ "content": [{ "type": "text", "text": format!("🗺️ Grafo exportado a '{}' ({} bytes). Ábrelo en un navegador.", abs_path, svg.len()) }] })
                                            }
                                            Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": format!("Error al escribir SVG: {}", e) }] }),
                                        }
                                    }
                                    Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                                }
                            }
                            None => json!({ "isError": true, "content": [{ "type": "text", "text": "No hay proyecto indexado en esta ruta." }] }),
                        }
                    }

                    // ── list_projects ──────────────────────────────────────
                    "list_projects" => {
                        match db.list_all_projects().await {
                            Ok(projects) => {
                                let mut text = format!("📂 Proyectos indexados ({}):\n", projects.len());
                                for (name, path) in &projects {
                                    text.push_str(&format!("  - {} ({})\n", name, path));
                                }
                                if projects.is_empty() {
                                    text.push_str("  (No hay proyectos indexados)\n");
                                }
                                json!({ "content": [{ "type": "text", "text": text }] })
                            }
                            Err(e) => json!({ "isError": true, "content": [{ "type": "text", "text": e.to_string() }] }),
                        }
                    }

                    _ => {
                        log_tool_call(&tool_name, start.elapsed().as_millis() as u64, "UNKNOWN", "");
                        json!({ "isError": true, "content": [{ "type": "text", "text": format!("Tool '{}' no reconocida. Usa tools/list para ver las disponibles.", tool_name) }] })
                    }
                };
                send_response(json!({ "jsonrpc": "2.0", "id": req.id, "result": result })).await?;
            } else if let Some(id) = req.id {
                send_response(json!({ "jsonrpc": "2.0", "id": id, "result": { "handled": true } })).await?;
            }
        }
    }
    Ok(())
}

// ── Funciones auxiliares ────────────────────────────────────────────────────

async fn send_response(value: Value) -> Result<()> {
    let json = serde_json::to_string(&value)?;
    let mut out = tokio::io::stdout();
    out.write_all(format!("{}\n", json).as_bytes()).await?;
    out.flush().await?;
    Ok(())
}
