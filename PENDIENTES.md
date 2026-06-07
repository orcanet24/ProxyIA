# Tareas Pendientes (ProxyIAv2)

Este documento registra las optimizaciones y características planificadas para futuras iteraciones del proyecto ProxyIAv2.

## 1. Caché Inteligente de Indexación (Hash Check)
**Estado:** ✅ Implementado
**Prioridad:** Alta

**Descripción del Problema:** 
Actualmente, por directiva estricta, Qwen está obligado a ejecutar `MANDATORY_index_project` cada vez que inicia una nueva sesión de código. Esto provoca que ProxyIAv2 vuelva a leer y vectorizar (mediante el servidor MiniLM externo) todos los archivos del proyecto, incluso aquellos que no han sufrido ninguna modificación desde el último escaneo. Esto gasta tiempo de cómputo innecesariamente.

**Solución Propuesta:**
Implementar una validación criptográfica (Hashing) para saltar la vectorización de archivos inalterados.

**Pasos de Implementación:**
1. **Actualizar DB (`db.rs` y `schema.sql`):**
   - Asegurarnos de que el campo `content_hash TEXT` existente en la base de datos sea leído y actualizado correctamente.
   - Modificar la función `get_node_by_path` para que devuelva también el `content_hash` actual guardado en base de datos.
2. **Lógica de Hashing (`scanner.rs`):**
   - Antes de enviar el contenido al `embedder.embed()`, calcular el hash SHA-256 (o similar) del contenido del archivo.
   - Comparar el nuevo hash con el hash almacenado en la base de datos para ese path.
3. **Decisión de Vectorización:**
   - Si el hash **coincide**, pasar `semantic_vector = None` a la función `insert_node`. Gracias al uso de `COALESCE` en nuestra query SQL de upsert, esto mantendrá el vector antiguo intacto en SQLite.
   - Si el hash **no coincide** (o es un archivo nuevo), llamar a `embedder.embed()`, generar el vector y guardarlo junto con su nuevo hash.

**Resultado Esperado:** 
La indexación recurrente pasará de tomar segundos/minutos a tomar milisegundos, convirtiendo verdaderamente la inicialización de Qwen en "el rayo".

## 2. Posicionamiento Lógico Espacial (Tree-sitter)
**Estado:** Pendiente / Propuesto (Junio 2026)
**Prioridad:** Alta

**Descripción del Problema:**
El `TreePositioner` actual calcula las coordenadas [X, Y] basándose puramente en la jerarquía de carpetas. Esto no refleja la realidad lógica del software, donde archivos en distintas carpetas pueden estar íntimamente relacionados por llamadas a funciones o herencia.

**Solución Propuesta:**
Sustituir la jerarquía de carpetas por un grafo de dependencias estático extraído localmente.

**Pasos de Implementación:**
1. **Integrar Tree-sitter:**
   - Añadir la dependencia `tree-sitter` y los parsers necesarios (`tree-sitter-rust`, etc.).
   - Crear un módulo de extracción de símbolos que identifique `imports`, `definitions` y `calls`.
2. **Cálculo de Proximidad:**
   - Implementar un algoritmo simple de atracción (Force-Directed) donde los archivos con más dependencias mutuas se "atraigan" en el eje X.
   - Mantener el eje Y para la jerarquía de "importancia" (raíces arriba, utilidades abajo).
3. **Refactorización de `scanner.rs`:**
   - Modificar `scan_recursive` para que primero recolecte todas las dependencias y luego asigne las posiciones finales.

**Resultado Esperado:**
Un mapa espacial del código que agrupa archivos por su función lógica, permitiendo que la búsqueda por proximidad (R-Tree) recupere contexto mucho más relevante.

---
*Nota: Ver `doc/memoria/vision_tecnica_v2.md` para el análisis detallado de esta mejora.*


---

## 3. Plan de AcciÃ³n EstratÃ©gico: "CirugÃ­a de Eficiencia" (Junio 2026)
**Estado:** Pendiente / URGENTE
**Prioridad:** CRÃTICA (Nivel 10)

Este plan consolida a ProxyIAv2 como la herramienta lÃ­der en ahorro de tokens mediante tres ejes de acciÃ³n inmediata:

### A. ModularizaciÃ³n "Zero-Overlap" (Limpieza de Arquitectura)
**Objetivo:** Dividir `main.rs` (70KB+) en mÃ³dulos especializados para facilitar la auditorÃ­a por IA y el mantenimiento humano.
- **AcciÃ³n:** Crear carpeta `src/mcp/` y `src/tools/`.
- **ImplementaciÃ³n:**
  1. Mover definiciones de tipos MCP a `src/mcp/types.rs`.
  2. Mover lÃ³gica de cada herramienta (`index_project`, `search_context`, `get_structural_map`) a archivos individuales en `src/tools/`.
  3. Reducir `main.rs` exclusivamente a la inicializaciÃ³n del servidor y el router de comandos.
- **Resultado:** Archivos de mÃ¡ximo 15-20KB que permiten una carga de contexto mÃ¡s quirÃºrgica.

### B. MÃ©trica de Valor: Reporte de ROI de Tokens (`get_roi_report`)
**Objetivo:** Demostrar el ahorro econÃ³mico tangible al usuario.
- **AcciÃ³n:** Crear una nueva herramienta MCP `proxyia_mcp.get_roi_report`.
- **ImplementaciÃ³n:**
  1. Extender `token_savings.log` para registrar no solo bytes, sino una estimaciÃ³n de tokens (ej: 1 token â‰ˆ 4 bytes).
  2. Implementar una funciÃ³n en `db.rs` que sume el total de tokens ahorrados vs. tokens que habrÃ­an costado leer los archivos completos.
  3. Formatear la salida en Markdown con: "Tokens ahorrados hoy", "Dinero estimado ahorrado ($ USD)", "Eficiencia de Contexto (%)".
- **Resultado:** El usuario ve el valor directo en su factura de API, asegurando la retenciÃ³n del producto.

### C. EvoluciÃ³n del Posicionamiento 3D (Clusters LÃ³gicos)
**Objetivo:** Refinar el `HashEmbedder` nativo para que la cercanÃ­a espacial sea 100% fiel a la arquitectura.
- **AcciÃ³n:** Integrar el anÃ¡lisis de `tree-sitter` (Tarea #2) con el posicionador 3D.
- **ImplementaciÃ³n:**
  1. En `positional.rs`, ajustar las coordenadas Z para que representen el "API Surface" (nodos pÃºblicos mÃ¡s altos, privados mÃ¡s profundos).
  2. Actualizar `exporter.rs` para que el SVG generado use colores basados en clusters lÃ³gicos (ej: todos los controladores en azul, modelos en verde).
  3. Optimizar el `LocalEmbedder` para usar un hashing semÃ¡ntico mÃ¡s denso (proyectando de 128D a 384D).
- **Resultado:** La IA podrÃ¡ navegar el cÃ³digo "por instinto espacial", encontrando dependencias ocultas sin buscarlas explÃ­citamente.

---
*Nota: Este plan debe ejecutarse siguiendo el ciclo ANALIZAR -> PLANIFICAR -> IMPLEMENTAR definido en GEMINI.md.*
