# ProxyIA MCP Server 🧠📂
# ProxyIA
**ProxyIA** es un servidor MCP (Model Context Protocol) que actúa como **memoria dual** para código fuente. Indexa proyectos una sola vez y permite a asistentes de IA buscar, explorar y entender el código **sin leer archivos completos**, ahorrando **~90% de tokens de contexto**.

> *"No leas el código completo. Deja que ProxyIA te dé solo lo que necesitas."*

---

## 🚀 Características

| Funcionalidad | Descripción | Ahorro de tokens |
|---|---|---|
| **Indexación semántica** | Escanea proyectos y genera vectores de 384D + resúmenes L1 | — |
| **Búsqueda contextual** | Encuentra fragmentos de código por relevancia semántica | ~95% |
| **Mapa estructural** | Resúmenes L1 de TODO el proyecto en una llamada | ~94% |
| **Exploración de vecinos** | Archivos relacionados por cercanía estructural | ~99% |
| **Indexación de funciones** | Extrae funciones individuales (tree-sitter, 11 lenguajes) | — |
| **Búsqueda de funciones** | Por nombre exacto o similitud semántica | ~90% |
| **Grafo de llamadas** | Detecta qué función llama a cuál otra (intra y cross-file) | — |
| **Watch mode** | Monitorea cambios en tiempo real con `notify` | — |
| **Export SVG** | Grafo posicional del proyecto como vectorial | — |
| **Posicionamiento 3D** | Coordenadas (x, y, z) que reflejan jerarquía y API surface | — |
| **Embeddings locales** | HashEmbedder: 384 dimensiones, cero dependencias externas | — |
| **Export/Import índice** | Portabilidad del índice entre máquinas | — |
| **Cleanup** | Limpia entradas huérfanas del índice | — |
| **Telemetría de ahorro** | Reporte de tokens ahorrados (opcional) | — |

---

## 📦 Instalación

### Opción 1: Binario precompilado (Windows)

1. Descarga `ProxyIA.exe` desde [Releases](https://github.com/tuusuario/ProxyIAv2/releases)
2. Colócalo en `C:\herramientas\proxyia\` (o cualquier carpeta)
3. Crea un archivo `.env` junto al `.exe`:

```env
# Opcional: LLM para resúmenes conceptuales (recomendado)
LLM_API_URL=http://localhost:11434/api/generate
LLM_API_KEY=ollama

# Opcional: reporte de ahorro de tokens
TOKEN_SAVINGS_LOG=true
```

4. Configura tu cliente MCP (ver [Configuración MCP](doc/CONFIGURACION_MCP.md))

### Opción 2: Compilar desde fuente

```bash
# Requiere Rust 1.75+
git clone https://github.com/tuusuario/ProxyIAv2.git
cd ProxyIAv2

# Compilación release
cargo build --release

# El binario estará en target/release/ProxyIA.exe
```

---

## ⚙️ Configuración del cliente MCP

ProxyIA implementa el protocolo MCP 2024-11-05. Se conecta vía **stdio** (entrada/salida estándar).

### claude-code (config.json)

```json
{
  "mcpServers": {
    "proxyia": {
      "command": "C:\\herramientas\\proxyia\\ProxyIA.exe",
      "args": []
    }
  }
}
```

### continue.dev (config.json)

```json
{
  "experimental": {
    "mcpServers": {
      "proxyia": {
        "command": "C:\\herramientas\\proxyia\\ProxyIA.exe",
        "args": []
      }
    }
  }
}
```

Para más clientes, ver [Configuración MCP](doc/CONFIGURACION_MCP.md).

---

## 🧠 Flujo de trabajo recomendado

### Primera vez: indexar el proyecto

```
→ tools/call: index_project { "path": "C:/MiProyecto" }
← "✅ 'MiProyecto' indexado con L1 Cache (2340 ms)."
```

### Después: siempre usar búsqueda contextual

```
→ tools/call: search_context { "query": "función que maneja autenticación" }
← "--- src/auth.rs [Sem: 0.92, Pos: 0.85, Comb: 0.89] ---
fn authenticate()..."
```

En lugar de leer el archivo completo (~500 tokens), recibes solo el fragmento relevante (~30 tokens). **Ahorro: ~94%.**

### Explorar vecinos de un archivo

```
→ tools/call: explore_neighbors { "file_path": "src/auth.rs" }
← "📂 Vecinos de 'src/auth.rs':
  - src/session.rs (x:23.45, y:12.30)
  - src/middleware.rs (x:25.10, y:14.22)"
```

### Mapa estructural completo

```
→ tools/call: get_structural_map { }
← "FILE: src/main.rs
SUMMARY: Programa principal. Define: mod db; mod scanner; ...
FILE: src/db.rs
SUMMARY: Gestor de base de datos SQLite..."
```

**Sin leer un solo archivo completo. Ahorro: ~94%.**

### Buscar funciones

```
→ tools/call: search_functions { "query": "authenticate", "by_semantic": false }
→ tools/call: search_functions { "query": "validar token jwt", "by_semantic": true }
```

### Watch mode (monitoreo en tiempo real)

```
→ tools/call: start_watcher { }
← "👀 Watch mode iniciado."
```

### Exportar SVG del grafo estructural

```
→ tools/call: export_svg { "output": "mi_proyecto.svg" }
← "🗺️ Grafo exportado a 'mi_proyecto.svg'."
```

Abre el SVG en un navegador para ver el mapa visual del proyecto.

---

## 🛠️ Herramientas MCP disponibles

| Herramienta | Descripción |
|---|---|
| `MANDATORY_index_project` / `index_project` | Indexa el proyecto (solo con permiso del usuario) |
| `MANDATORY_search_context` / `search_context` | Búsqueda semántica de fragmentos de código |
| `explore_neighbors` | Archivos relacionados por cercanía estructural |
| `get_structural_map` | Resúmenes L1 de todo el proyecto |
| `search_semantic_summaries` | Busca resúmenes similares semánticamente |
| `search_functions` | Busca funciones por nombre o semántica |
| `start_watcher` | Monitorea cambios en tiempo real |
| `export_svg` | Exporta grafo posicional como SVG |
| `get_token_savings_report` | Estadísticas de ahorro de tokens |
| `cleanup_index` | Limpia entradas huérfanas |
| `export_index` | Exporta índice a JSON portátil |
| `import_index` | Importa índice desde JSON |
| `list_projects` | Lista proyectos indexados |

---

## 🗺️ Export SVG — Mapa visual del proyecto

El grafo posicional se exporta como SVG vectorial:

- **Nodos**: cada archivo es un rectángulo con su nombre
- **Aristas**: líneas que conectan archivos con dependencias
- **Profundidad (Z)**: mapeada a opacidad — archivos con más funciones públicas (mayor Z) aparecen más visibles
- **Leyenda**: explica la codificación visual

![Ejemplo de grafo SVG](doc/proxyia_graph_example.svg)

---

## 🔧 Arquitectura

```
src/
├── main.rs        → Servidor MCP (event loop, handlers)
├── db.rs          → DatabaseManager (SQLite + sqlx)
├── scanner.rs     → FileScanner (escaneo, tree-sitter, sumarización)
├── positional.rs  → Position3D (force-directed layout 3D)
├── embeddings.rs  → EmbeddingEngine (trait)
├── local_embed.rs → HashEmbedder (384 dimensiones, cero deps)
├── search.rs      → HybridSearcher (búsqueda dual: semántica + posicional)
├── llm.rs         → LlmClient (opcional, para resúmenes conceptuales)
├── watcher.rs     → Watch mode (notify)
└── exporter.rs    → Export SVG del force-directed layout
```

---

## 📊 Telemetría (opcional)

Activa en `.env`:

```env
TOKEN_SAVINGS_LOG=true
```

Cada llamada a `search_context`, `get_structural_map` o `search_semantic_summaries` registra:
- Tokens que se habrían usado si leyeras archivos completos
- Tokens realmente enviados
- Tokens ahorrados y porcentaje

Consulta con:

```
→ tools/call: get_token_savings_report { }
```

---

## 📁 SQLite — Base de datos

ProxyIA usa una sola base de datos SQLite (`kairos.db`) que almacena:
- `projects` — proyectos indexados
- `filesystem_tree` — nodos (archivos) con vectores, posiciones y resúmenes
- `dependencies` — aristas (dependencias entre archivos)
- `functions` — funciones individuales extraídas por tree-sitter
- `function_calls` — grafo de llamadas entre funciones

La DB es **persistente**: una vez indexado un proyecto, los datos sobreviven entre sesiones.

---

## 🤝 Contribuir

1. Fork el repositorio
2. Crea una rama: `git checkout -b feature/nueva-funcionalidad`
3. Haz tus cambios
4. Build: `cargo build --release`
5. Push y PR

### Áreas para contribuir
- Re-indexación automática en watch mode
- Más lenguajes en el extractor de funciones (tree-sitter)
- Interfaz web para visualización del grafo
- Tests automatizados
- Soporte para macOS/Linux

---

## 📄 Licencia

MIT License — ver [LICENSE](LICENSE) para detalles.

---

## 🙏 Agradecimientos

- [Model Context Protocol](https://modelcontextprotocol.io) — estándar de comunicación IA↔herramientas
- [Tree-sitter](https://tree-sitter.github.io/tree-sitter/) — parsing incremental de código fuente
- [notify](https://github.com/notify-rs/notify) — watchdog multiplataforma
- [sqlx](https://github.com/launchbadge/sqlx) — driver SQLite asíncrono

---

> **ProxyIA** — Memoria dual para código. Indexa una vez, busca siempre. 🧠
