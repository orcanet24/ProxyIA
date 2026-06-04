// src/scanner.rs
// MEJORADO v2.1: Soporte multi-lenguaje (Python, Go, Java, C++, C#, Ruby),
// detección de llamadas a funciones, análisis de dependencias más profundo.
//
// Tree-sitter ahora detecta:
//   - imports/uses/requires (todos los lenguajes)
//   - llamadas a funciones (function_call, method_invocation)
//   - definiciones de clases/funciones/interfaces
//   - herencia e implementación de interfaces

use anyhow::Result;
use std::path::Path;
use crate::db::DatabaseManager;
use crate::embeddings::EmbeddingEngine;
use crate::positional::{StablePositioner, ForceDirectedLayout, Position3D};
use std::fs;
use std::sync::Arc;
use std::collections::HashMap;
use tree_sitter::{Parser, Query, QueryCursor, Language};
use sha2::{Sha256, Digest};

/// Normaliza paths para consistencia cross-platform: quita prefijo \\?\
fn norm(p: &std::path::Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.trim_start_matches("//?/").trim_start_matches("\\\\?\\").to_string()
}

pub struct UniversalDistiller {}

impl UniversalDistiller {
    pub fn new() -> Self {
        Self {}
    }

    pub fn skeletonize(&self, path: &Path) -> Result<String> {
        let content = fs::read_to_string(path)?;
        let mut skeleton = String::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() { continue; }

            let indent = line.len() - line.trim_start().len();

            if trimmed.starts_with("///") || trimmed.starts_with("/**") || trimmed.starts_with("# ") {
                skeleton.push_str(line);
                skeleton.push_str("\n");
                continue;
            }

            let is_potential_signature = indent <= 4 && (
                trimmed.ends_with('{') ||
                trimmed.ends_with(':') ||
                trimmed.starts_with("pub ") ||
                trimmed.starts_with("fn ") ||
                trimmed.starts_with("class ") ||
                trimmed.starts_with("def ") ||
                trimmed.starts_with("interface ")
            );

            if is_potential_signature {
                skeleton.push_str(line);
                if trimmed.ends_with('{') {
                    skeleton.push_str(" ... }\n");
                } else if trimmed.ends_with(':') {
                    skeleton.push_str(" ...\n");
                } else {
                    skeleton.push_str("\n");
                }
            } else if indent == 0 && (trimmed.starts_with("use") || trimmed.starts_with("import")) {
                skeleton.push_str(line);
                skeleton.push_str("\n");
            }
        }
        Ok(skeleton)
    }
}

/// Configuración de análisis AST para cada lenguaje
struct LangConfig {
    language: Language,
    /// Query para capturar imports y referencias a otros archivos
    import_query: &'static str,
    /// Query para capturar llamadas a funciones (opcional)
    call_query: Option<&'static str>,
}

/// Obtiene la configuración tree-sitter para un lenguaje dado
fn get_lang_config(lang: &str) -> Option<LangConfig> {
    match lang {
        "rust" => Some(LangConfig {
            language: tree_sitter_rust::language(),
            import_query: "(use_declaration) @import (mod_item) @mod",
            call_query: Some("(call_expression) @call"),
        }),
        "php" => Some(LangConfig {
            language: tree_sitter_php::language_php(),
            import_query: "(namespace_definition) @ns (use_declaration) @use",
            call_query: Some("(function_call_expression) @call (method_call_expression) @method_call"),
        }),
        "javascript" | "typescript" => Some(LangConfig {
            language: tree_sitter_javascript::language(),
            import_query: "(import_statement) @import (require_function) @require",
            call_query: Some("(call_expression) @call"),
        }),
        "python" => Some(LangConfig {
            language: tree_sitter_python::language(),
            import_query: "(import_statement) @import (import_from_statement) @import_from",
            call_query: Some("(call) @call"),
        }),
        "go" => Some(LangConfig {
            language: tree_sitter_go::language(),
            import_query: "(import_declaration) @import",
            call_query: Some("(call_expression) @call"),
        }),
        "java" => Some(LangConfig {
            language: tree_sitter_java::language(),
            import_query: "(import_declaration) @import",
            call_query: Some("(method_invocation) @call"),
        }),
        "cpp" | "c" => Some(LangConfig {
            language: tree_sitter_cpp::language(),
            import_query: "(preproc_include) @include (using_declaration) @using (namespace_definition) @ns",
            call_query: Some("(call_expression) @call"),
        }),
        "csharp" => Some(LangConfig {
            language: tree_sitter_c_sharp::language(),
            import_query: "(using_directive) @using",
            call_query: Some("(invocation_expression) @call"),
        }),
        "ruby" => Some(LangConfig {
            language: tree_sitter_ruby::language(),
            import_query: "(require) @require",
            call_query: Some("(call) @call"),
        }),
        _ => None,
    }
}

pub struct FileScanner {
    pub db: DatabaseManager,
    pub embedder: Option<Arc<dyn EmbeddingEngine>>,
    pub llm: Option<Arc<crate::llm::LlmClient>>,
    pub distiller: UniversalDistiller,
}

impl FileScanner {
    pub fn new(db: DatabaseManager, embedder: Option<Arc<dyn EmbeddingEngine>>, llm: Option<Arc<crate::llm::LlmClient>>) -> Self {
        Self {
            db,
            embedder,
            llm,
            distiller: UniversalDistiller::new()
        }
    }

    pub async fn scan_project(&self, project_id: i64, root_path: &Path) -> Result<()> {
        let root_name = root_path.file_name().unwrap_or_default().to_string_lossy();
        let root_pos = StablePositioner::root_position();
        let root_path_s = norm(root_path);
        let root_db_id = self.db.insert_node(project_id, &root_name, &root_path_s, None, 0, root_pos, "directory", None, None, None, None).await?;

        // Primera pasada: escanear todos los archivos con posiciones base (hash)
        self.scan_recursive(project_id, root_path, root_db_id, 1, root_pos).await?;

        // Segunda pasada: analizar dependencias (imports + llamadas a funciones)
        self.analyze_all_dependencies_deep(project_id).await?;

        // Tercera pasada: force-directed layout basado en dependencias
        self.apply_force_directed_layout(project_id).await?;

        Ok(())
    }

    async fn scan_recursive(&self, project_id: i64, path: &Path, parent_id: i64, depth: i32, _parent_pos: Position3D) -> Result<()> {
        let entries: Vec<_> = match fs::read_dir(path) {
            Ok(dir) => dir.filter_map(|e| e.ok()).collect(),
            Err(_) => {
                // Directorio no accesible → skip silencioso (no fatal)
                eprintln!("⚠️ Directorio no accesible: {}", path.display());
                return Ok(());
            }
        };

        for entry in entries.into_iter() {
            let entry_path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry_path.is_dir();
            let lang = self.detect_language(&entry_path);

            // Posición base por hash (estable, determinística)
            let dir_path = path.to_string_lossy();
            let full_path = entry_path.to_string_lossy();
            // Inicialmente public_func_count = 0; se actualiza tras indexar funciones
            let position = StablePositioner::calculate_base(&dir_path, &full_path, depth, 0);

            let mut semantic_vector = None;
            let mut current_hash = None;
            let mut structural_summary = None;

            if !is_dir && lang.is_some() && lang.as_deref() != Some("unknown") {
                if let Ok(content) = fs::read_to_string(&entry_path) {
                    let hash = format!("{:x}", Sha256::digest(content.as_bytes()));
                    current_hash = Some(hash.clone());

                    // Obtener hash anterior para decidir qué regenerar
                    let entry_path_norm = norm(&entry_path);
                    let old_hash = self.db.get_node_hash(&entry_path_norm).await.unwrap_or(None);
                    let file_changed = old_hash.as_deref() != Some(&hash);

                    if file_changed {
                        // Generar structural_summary: LLM o skeletonizer
                        if let Some(llm) = &self.llm {
                            match llm.summarize_file(&entry_path.to_string_lossy(), &content).await {
                                Ok(summary) => {
                                    structural_summary = Some(summary);
                                }
                                Err(e) => {
                                    eprintln!("⚠️ LLM summarization falló para '{}': {}. Usando skeletonizer.", entry_path.display(), e);
                                    if let Ok(skeleton) = self.distiller.skeletonize(&entry_path) {
                                        structural_summary = Some(skeleton);
                                    }
                                }
                            }
                        } else {
                            if let Ok(skeleton) = self.distiller.skeletonize(&entry_path) {
                                structural_summary = Some(skeleton);
                            }
                        }

                        // Generar nuevo embedding solo si cambió el archivo
                        if let Some(embedder) = &self.embedder {
                            let trimmed = content.trim();
                            if !trimmed.is_empty() {
                                match embedder.embed(trimmed).await {
                                    Ok(vector) => {
                                        semantic_vector = Some(vector);
                                    }
                                    Err(e) => {
                                        eprintln!("⚠️ Embedding falló para '{}': {}. Se usará vector anterior si existe.", entry_path.display(), e);
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let path_norm = norm(&entry_path);

            let node_id = self.db.insert_node(
                project_id, &name, &path_norm, Some(parent_id), depth, position,
                if is_dir { "directory" } else { "file" }, lang.as_deref(), semantic_vector, current_hash.as_deref(), structural_summary.as_deref()
            ).await?;

            // ── Indexar funciones individuales del archivo ──────────────
            if !is_dir && lang.is_some() && lang.as_deref() != Some("unknown") {
                if let Ok(content) = fs::read_to_string(&entry_path) {
                    let lang_str = lang.as_deref().unwrap_or("unknown");
                    let config = get_lang_config(lang_str);

                    if let Some(cfg) = config {
                        let mut parser = Parser::new();
                        if parser.set_language(&cfg.language).is_ok() {
                            let func_query = match lang_str {
                                "rust" => Some("(function_item name: (identifier) @name) @func"),
                                "python" => Some("(function_definition name: (identifier) @name) @func"),
                                "go" => Some("(function_declaration name: (identifier) @name) @func"),
                                "java" => Some("(method_declaration name: (identifier) @name) @func"),
                                "cpp" | "c" => Some("(function_definition declarator: (function_declarator declarator: (identifier) @name)) @func"),
                                "csharp" => Some("(method_declaration name: (identifier) @name) @func"),
                                "ruby" => Some("(method name: (identifier) @name) @func"),
                                "php" => Some("(function_definition name: (name) @name) @func (method_declaration name: (name) @name) @method"),
                                "javascript" | "typescript" => Some("(function_declaration name: (identifier) @name) @func"),
                                _ => None,
                            };

                            if let Some(fq) = func_query {
                                if let Ok(query) = Query::new(&cfg.language, fq) {
                                    let mut cursor = QueryCursor::new();
                                    if let Some(tree) = parser.parse(&content, None) {
                                        for m in cursor.matches(&query, tree.root_node(), content.as_bytes()) {
                                            let captures: Vec<_> = m.captures.iter().collect();
                                            let name_cap = captures.iter().find(|c|
                                                query.capture_names()[c.index as usize] == "name"
                                            );

                                            if let Some(nc) = name_cap {
                                                let name = &content[nc.node.start_byte()..nc.node.end_byte()];
                                                let start_line = content[..nc.node.start_byte()].lines().count() as i32 + 1;

                                                // Detectar si es pública
                                                let pre_text = &content[..nc.node.start_byte()];
                                                let is_public = pre_text.lines().last().map(|l|
                                                    l.trim().starts_with("pub ") || l.trim() == "pub"
                                                ).unwrap_or(false);

                                                // Firma = primera línea del nodo función
                                                let func_text = &content[nc.node.start_byte()..nc.node.end_byte()];
                                                let signature = func_text.lines().next().unwrap_or("").trim().to_string();

                                                // Embedding del nombre de la función
                                                let func_vec = if let Some(embedder) = &self.embedder {
                                                    if !name.is_empty() {
                                                        embedder.embed(name).await.ok()
                                                    } else { None }
                                                } else { None };

                                                let func_hash = format!("{:x}", Sha256::digest(name.as_bytes()));

                                                let _ = self.db.insert_function(
                                                    node_id, project_id, name, &signature,
                                                    start_line, start_line + 1,
                                                    &path_norm, "function", is_public,
                                                    func_vec, Some(&func_hash),
                                                ).await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ── Recursión en subdirectorios ──
            if is_dir && name != ".git" && name != "target" && name != "node_modules" && name != "backups" && name != "vendor" && name != "storage" {
                Box::pin(self.scan_recursive(project_id, &entry_path, node_id, depth + 1, position)).await?;
            }
        }
        Ok(())
    }

    /// Analiza dependencias: imports + llamadas a funciones.
    /// Para cada archivo, extrae qué otros archivos del proyecto referencia.
    async fn analyze_all_dependencies_deep(&self, project_id: i64) -> Result<()> {
        let nodes = self.db.get_all_nodes(project_id).await?;
        let mut path_map = HashMap::new();
        let mut name_to_id: HashMap<String, Vec<i64>> = HashMap::new();
        for (id, path_str, _) in &nodes {
            path_map.insert(path_str.clone(), *id);
            if let Some(stem) = Path::new(path_str).file_stem().and_then(|s| s.to_str()) {
                name_to_id.entry(stem.to_lowercase()).or_default().push(*id);
            }
        }

        // ── Cargar funciones indexadas para grafo de llamadas ──────────
        // Mapa: (func_name_lower, file_node_id) → func_id
        let all_funcs = self.db.get_all_functions(project_id).await?;
        let mut func_name_by_file: HashMap<(String, i64), Vec<(i64, String)>> = HashMap::new();
        // También un mapa directo nombre → lista de func_id (para lookup sin file context)
        let mut func_name_global: HashMap<String, Vec<(i64, String)>> = HashMap::new();
        for (func_id, func_name, sig, func_path, _start, _end, _blob) in &all_funcs {
            let name_lower = func_name.to_lowercase();
            // Obtener el node_id del archivo contenedor
            if let Some((node_id, _)) = self.db.get_node_by_path(func_path).await.ok().flatten() {
                func_name_by_file
                    .entry((name_lower.clone(), node_id))
                    .or_default()
                    .push((*func_id, sig.clone()));
            }
            func_name_global
                .entry(name_lower)
                .or_default()
                .push((*func_id, sig.clone()));
        }

        let mut parser = Parser::new();

        for (id, path_str, _) in &nodes {
            let path = Path::new(path_str);
            let lang_str = match self.detect_language(path) {
                Some(l) => l,
                None => continue,
            };

            let config = match get_lang_config(&lang_str) {
                Some(c) => c,
                None => continue,
            };

            parser.set_language(&config.language)?;
            if let Ok(content) = fs::read_to_string(path) {
                if let Some(tree) = parser.parse(&content, None) {
                    // 1. Analizar imports (referencias explícitas a otros archivos)
                    if let Ok(query) = Query::new(&config.language, config.import_query) {
                        let mut cursor = QueryCursor::new();
                        let matches = cursor.matches(&query, tree.root_node(), content.as_bytes());

                        for m in matches {
                            for capture in m.captures {
                                let text = &content[capture.node.start_byte()..capture.node.end_byte()];
                                let tokens: Vec<&str> = text.split(|c: char|
                                    c.is_whitespace() || "::;{},./'\"()[]".contains(c)
                                ).filter(|t| !t.is_empty()).collect();

                                for (tpath, tid) in &path_map {
                                    let t_name = Path::new(tpath).file_stem().and_then(|s| s.to_str()).unwrap_or("");
                                    if t_name.is_empty() || tid == id { continue; }
                                    if tokens.iter().any(|t| *t == t_name) {
                                        let _ = self.db.insert_dependency(*id, *tid, "import").await;
                                    }
                                }
                            }
                        }
                    }

                    // 2. Analizar llamadas a funciones (referencias a métodos/funciones de otros archivos)
                    if let Some(call_query_str) = config.call_query {
                        if let Ok(call_query) = Query::new(&config.language, call_query_str) {
                            let mut cursor = QueryCursor::new();
                            let call_matches = cursor.matches(&call_query, tree.root_node(), content.as_bytes());

                            for m in call_matches {
                                for capture in m.captures {
                                    let text = &content[capture.node.start_byte()..capture.node.end_byte()];
                                    let tokens: Vec<&str> = text.split(|c: char|
                                        c.is_whitespace() || "::;{},./'\"()[]".contains(c)
                                    ).filter(|t| !t.is_empty()).collect();

                                    for token in &tokens {
                                        let token_lower = token.to_lowercase();
                                        if let Some(ids) = name_to_id.get(&token_lower) {
                                            for &tid in ids {
                                                if tid == *id { continue; }
                                                let _ = self.db.insert_dependency(*id, tid, "call").await;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    // 3. Grafo de llamadas entre FUNCIONES ────────────────
                    // Vincula llamadas a nombres de funciones indexadas,
                    // tanto intra-file como cross-file.
                    if let Some(call_query_str) = config.call_query {
                        if let Ok(call_query) = Query::new(&config.language, call_query_str) {
                            let mut cursor = QueryCursor::new();
                            let call_matches = cursor.matches(&call_query, tree.root_node(), content.as_bytes());

                            for m in call_matches {
                                for capture in m.captures {
                                    let text = &content[capture.node.start_byte()..capture.node.end_byte()];
                                    let tokens: Vec<&str> = text.split(|c: char|
                                        c.is_whitespace() || "::;{},./'\"()[]".contains(c)
                                    ).filter(|t| !t.is_empty()).collect();

                                    for token in &tokens {
                                        let token_lower = token.to_lowercase();

                                        // Vincular a funciones del MISMO archivo
                                        // Cada token que coincide con una función indexada en este archivo
                                        // es probablemente un uso/llamada de esa función
                                        if let Some(func_ids) = func_name_by_file.get(&(token_lower.clone(), *id)) {
                                            for (func_id, _) in func_ids {
                                                let _ = self.db.insert_function_dependency(
                                                    *func_id, *func_id, "intra_call"
                                                ).await;
                                            }
                                        }

                                        // Vincular a funciones en OTRO archivo (cross-file)
                                        if let Some(global_matches) = func_name_global.get(&token_lower) {
                                            for (target_func_id, _) in global_matches {
                                                // Verificar que la función objetivo NO esté en el mismo archivo
                                                let is_same_file = func_name_by_file
                                                    .get(&(token_lower.clone(), *id))
                                                    .map(|same| same.iter().any(|(sfid, _)| sfid == target_func_id))
                                                    .unwrap_or(false);

                                                if !is_same_file {
                                                    let _ = self.db.insert_function_dependency(
                                                        *id, *target_func_id, "cross_call"
                                                    ).await;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        drop(parser);
        Ok(())
    }

    async fn apply_force_directed_layout(&self, project_id: i64) -> Result<()> {
        let nodes = self.db.get_all_nodes(project_id).await?;
        let dependencies = self.db.get_project_dependencies(project_id).await?;

        if nodes.is_empty() {
            return Ok(());
        }

        // Construir mapa de posiciones base (ID → Position3D)
        let mut base_positions = HashMap::new();
        for (id, _path_str, pos_bytes) in &nodes {
            if let Some(pos) = Position3D::from_bytes(pos_bytes) {
                base_positions.insert(*id, pos);
            }
        }

        // Aplicar force-directed layout sobre X,Y (mejorado: ahora refina ambos ejes)
        let layout = ForceDirectedLayout::default();
        let refined = layout.refine(&base_positions, &dependencies);

        // Actualizar posiciones en DB
        for (id, new_pos) in &refined {
            if let Some(base_pos) = base_positions.get(id) {
                let dx = (new_pos.x - base_pos.x).abs();
                let dy = (new_pos.y - base_pos.y).abs();
                if dx > 0.001 || dy > 0.001 {
                    self.db.update_node_position(*id, *new_pos).await?;
                }
            }
        }

        Ok(())
    }

    pub fn detect_language(&self, path: &Path) -> Option<String> {
        let file_name = path.file_name()?.to_string_lossy().to_lowercase();
        if file_name.ends_with(".blade.php") { return Some("blade".to_string()); }
        path.extension().and_then(|ext| ext.to_str()).map(|ext| match ext.to_lowercase().as_str() {
            "rs" => "rust",
            "py" => "python",
            "php" => "php",
            "go" => "go",
            "js" | "jsx" => "javascript",
            "ts" | "tsx" => "typescript",
            "java" => "java",
            "cpp" | "cc" | "cxx" | "hpp" | "h" => "cpp",
            "cs" => "csharp",
            "rb" => "ruby",
            "json" => "json",
            "md" => "markdown",
            "sql" => "sql",
            "yml" | "yaml" => "yaml",
            _ => "unknown",
        }.to_string())
    }

    pub async fn embed_query(&self, query: &str) -> Result<Vec<f32>> {
        if let Some(embedder) = &self.embedder {
            embedder.embed(query).await
        } else {
            Err(anyhow::anyhow!("No se ha configurado un motor de embeddings"))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn test_detect_language() {
        // Test de detect_language como función libre — no necesita DB
        let test_cases = vec![
            ("main.rs", Some("rust")),
            ("app.py", Some("python")),
            ("server.go", Some("go")),
            ("Main.java", Some("java")),
            ("index.js", Some("javascript")),
            ("component.tsx", Some("typescript")),
            ("Program.cs", Some("csharp")),
            ("model.rb", Some("ruby")),
            ("helper.cpp", Some("cpp")),
            ("module.h", Some("cpp")),
            ("README.md", Some("markdown")),
            ("config.json", Some("json")),
            ("unknown.xyz", Some("unknown")),
        ];

        for (filename, expected) in test_cases {
            let path = Path::new(filename);
            let ext = path.extension().and_then(|e| e.to_str()).map(|e| match e.to_lowercase().as_str() {
                "rs" => "rust",
                "py" => "python",
                "go" => "go",
                "js" | "jsx" => "javascript",
                "ts" | "tsx" => "typescript",
                "java" => "java",
                "cpp" | "cc" | "cxx" | "hpp" | "h" => "cpp",
                "cs" => "csharp",
                "rb" => "ruby",
                "json" => "json",
                "md" => "markdown",
                "sql" => "sql",
                "yml" | "yaml" => "yaml",
                _ => "unknown",
            });
            assert_eq!(ext, expected, "Fallo para '{}'", filename);
        }
    }
}