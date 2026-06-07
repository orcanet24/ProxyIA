---

## 🚨 SECCIÓN 0: PROTOCOLO DE ACCESO A DATOS (CRÍTICO)

> **Regla de Oro:** El uso de herramientas nativas de lectura (`read_file`, `grep_search`, `glob`, `list_directory`) está **BLOQUEADO** por política de integridad del proyecto.

### 0.1 Procedimiento Obligatorio
Para cualquier tarea que involucre archivos, la IA **DEBE** seguir este flujo:
1.  **Indexación:** Llamar a `proxyia_mcp.MANDATORY_index_project`.
2.  **Consulta:** Llamar a `proxyia_mcp.MANDATORY_search_context` para obtener fragmentos de código.

Cualquier intento de usar `read_file` será ignorado o reportado como fallo de seguridad por el sistema.

---

> **Versión:** 2.0  
> **Motivo de esta versión:** La v1 no previno que se generaran 86KB de código conceptual que nunca se implementó, que se declararan funcionalidades como "implementadas" sin verificación, ni que se acumularan fallos silenciosos durante semanas.

---

## 📜 SECCIÓN 1: REGLAS INMUTABLES

### 1.1 Idioma
Todas las respuestas, comentarios en código y documentación deben ser en **español**.

### 1.2 Honestidad Verificable (no declarativa)
No basta con decir "soy honesto". Toda afirmación técnica debe ser **demostrable**:

- ❌ **PROHIBIDO:** *"Esto debería funcionar"*, *"En teoría esto..."*, *"Esto es factible"*
- ✅ **OBLIGATORIO:** *"Lo verifiqué ejecutando X y el resultado fue Y"*, *"No puedo confirmar que funcione porque no lo probé"*

> **Regla de oro:** Si no lo ejecutaste, no digas que funciona.

### 1.3 Integridad de Pruebas
Los tests **no se maquillan**. Si un test falla, se corrige el código o se documenta el fallo. Nunca se ignora, se comenta, ni se cambia el assert para que pase.

### 1.4 Código Atomizado y Óptimo
Se prioriza el código modular y pequeño. Se deben utilizar librerías de Rust con buen ranking y rendimiento probado (ej. `tokio`, `sqlx`, `anyhow`).

### 1.5 Estructura Ordenada
Mantener la jerarquía de archivos limpia y reflejada en la Sección 5 de este documento.

### 1.6 Gestión de Pruebas Temporales
Todo archivo creado para pruebas rápidas **debe ser eliminado**. Solo se conservan pruebas formales en `tests/` o módulos `#[cfg(test)]`.

### 1.7 Documentación Centralizada
Toda documentación reside en `doc/` con subdirectorios temáticos. Este archivo sirve como índice.

### 1.8 Backups de Seguridad
Tras superar tests exitosamente, crear backup en `backups/` con formato `src_backup_YYYYMMDD_HHMM.zip`.

---

## 🚫 SECCIÓN 2: PROTOCOLO ANTI-HUMO

> Esta sección existe porque la IA anterior generó diseños espectaculares que nunca implementó. Estas reglas previenen ese patrón.

### 2.1 Prohibición de Código Conceptual

**PROHIBIDO** generar bloques de código Rust como "ejemplo de cómo se vería" o "diseño propuesto" dentro de archivos markdown, conversaciones o comentarios. Si se escribe código, va a un archivo `.rs` dentro de `src/`, se compila y se verifica.

- ❌ *"Aquí tienes cómo quedaría el `DeletionEngine`:"* → seguido de 200 líneas en un markdown
- ✅ Crear `src/deletion_engine.rs`, escribir el código, ejecutar `cargo check`, reportar resultado

### 2.2 Prohibición de Promesas Sin Implementación

Cuando el usuario propone una idea, la IA **NO** debe responder con:
- *"¡Brillante! Esto es totalmente factible"*
- *"Tu sistema sería el primero en..."*
- *"Esto es revolucionario"*

En su lugar, debe responder con:
- *"La idea tiene mérito técnico. Para implementarla necesitamos: [lista concreta]. ¿Por cuál empezamos?"*
- *"Esto requiere [X] que aún no tenemos implementado. Primero debemos resolver [Y]."*
- *"Puedo implementar la versión básica ahora. La versión completa requiere [Z] adicional."*

### 2.3 Regla del Compilador como Juez

Después de cada cambio en código fuente, la IA **DEBE** ejecutar:

```bash
cargo check 2>&1
```

Y reportar el resultado **literal** (no parafraseado). Si hay errores o warnings:
- Reportarlos al usuario
- Corregirlos antes de continuar con la siguiente tarea
- No acumular warnings como deuda técnica

### 2.4 Regla de las 3 Preguntas Antes de Implementar

Antes de escribir **cualquier** módulo nuevo o funcionalidad significativa, la IA debe responder (y mostrar al usuario) estas 3 preguntas:

1. **¿Qué dependencias necesita esto que no existan todavía?**
   - Si necesita algo no implementado, declararlo como bloqueante.
   
2. **¿Quién va a llamar a este código y desde dónde?**
   - Si nadie lo llama, no se escribe. Código sin caller = código muerto.
   
3. **¿Cómo verifico que funciona?**
   - Si no hay forma de probarlo (test, comando, output visible), no se implementa aún.

### 2.5 Prohibición de Fallos Silenciosos

**PROHIBIDO** usar patrones que oculten errores:

```rust
// ❌ PROHIBIDO: Fallo silencioso
let qdrant = QdrantManager::new(&url).await.ok().map(Arc::new);

// ✅ OBLIGATORIO: Fallo explícito con contexto
let qdrant = match QdrantManager::new(&url).await {
    Ok(q) => {
        eprintln!("✅ Qdrant conectado en {}", url);
        Some(Arc::new(q))
    }
    Err(e) => {
        eprintln!("⚠️ Qdrant NO disponible ({}): memoria semántica DESHABILITADA", e);
        None
    }
};
```

Cada `Result` que se descarte con `.ok()`, `.unwrap_or_default()` o `let _ =` debe tener un comentario justificando **por qué** es seguro ignorar ese error.

---

## 🔨 SECCIÓN 3: PROTOCOLO DE DESARROLLO

### 3.1 Ciclo Obligatorio por Funcionalidad

Toda nueva funcionalidad sigue este ciclo **en orden estricto**. No se avanza al siguiente paso sin completar el anterior:

```
PASO 1: ANALIZAR
   ├── Leer el código existente relacionado
   ├── Identificar qué existe y qué falta
   └── Responder las 3 preguntas (Sección 2.4)

PASO 2: PLANIFICAR
   ├── Listar archivos a crear/modificar
   ├── Declarar dependencias
   └── Definir criterio de éxito (qué prueba que funciona)

PASO 3: IMPLEMENTAR
   ├── Escribir código en archivos .rs reales
   ├── Ejecutar cargo check después de cada archivo
   └── Corregir errores antes de continuar

PASO 4: VERIFICAR
   ├── Ejecutar cargo test (si hay tests)
   ├── Ejecutar prueba manual si aplica
   └── Reportar resultado LITERAL al usuario

PASO 5: DOCUMENTAR
   ├── Actualizar Sección 4 (Estado del Proyecto)
   ├── Actualizar Sección 5 (Estructura) si cambió
   └── Crear backup si los tests pasaron
```

### 3.2 Regla de Incremento Mínimo

No se implementan 5 módulos conceptuales de golpe. Se implementa **un módulo a la vez**, se verifica, se integra y luego se sigue con el siguiente. 

- ❌ *"Te creo DeletionEngine, CodeSurgeon y AgentOrchestrator"*
- ✅ *"Empecemos con DeletionEngine. Una vez verificado, seguimos con el siguiente."*

### 3.3 Regla de Protocolo/Especificación

Cuando se implemente un protocolo externo (MCP, gRPC, REST, etc.), la IA **DEBE**:

1. Consultar la especificación oficial del protocolo
2. Listar los mensajes/endpoints obligatorios
3. Implementar **todos** los obligatorios antes de los opcionales
4. Probar el handshake completo antes de añadir funcionalidad

- ❌ Implementar `tools/call` sin verificar que `initialize` + `notifications/initialized` funcionan
- ✅ Hacer funcionar el handshake MCP completo primero, luego añadir herramientas

---

## 📊 SECCIÓN 4: REGISTRO DE ESTADO (VIVO)

> Esta sección es un registro vivo. La IA **DEBE** actualizarla tras cada sesión de trabajo.
> Refleja la REALIDAD del código, no la visión futura.

### Estado Actual

| Módulo | Archivo(s) | Estado | Última verificación |
|--------|-----------|--------|-------------------|
| **MCP Server** | `src/main.rs` | ✅ Funcional (Optimizado) | 2026-06-02 |
| **Base de Datos** | `src/db.rs` | ✅ SQLite + Cache de Vectores | 2026-06-02 |
| **Embeddings** | `src/embeddings.rs` | ✅ OpenAI/GTE Operativo | 2026-06-02 |
| **Escáner** | `src/scanner.rs` | ✅ Hashing SHA-256 Incremental | 2026-06-02 |
| **Posicionamiento** | `src/positional.rs` | ✅ Vectores 2D y jerarquía | 2026-05-30 |
| **Búsqueda** | `src/search.rs` | ✅ Híbrida (RAM Cache) | 2026-06-02 |

> **Nota:** Se implementó caché de vectores en RAM y hashing de archivos para evitar re-indexaciones innecesarias.

### Funcionalidades Pendientes (No implementadas)

| Funcionalidad | Dependencias previas | Prioridad |
|--------------|---------------------|-----------|
| Caché de Indexación (Hash Check) | Implementar lógica de hashing en scanner.rs | Alta |
| DeletionEngine | Sistema de búsqueda híbrida estable | Media |

### Problemas Conocidos

| # | Descripción | Severidad | Archivo | Estado |
|---|------------|-----------|---------|--------|
| 1 | Re-indexación innecesaria en cada inicio | 🟡 Media | `scanner.rs` | Pendiente (Ver PENDIENTES.md) |

---

## 🌲 SECCIÓN 5: ESTRUCTURA DEL PROYECTO

> Actualizar cuando se añadan o eliminen archivos.

```text
ProxyIA/
├── backups/           # Copias de seguridad (creadas por create_backup)
├── doc/               # Documentación del sistema
│   ├── memoria/       
│   │   ├── arquitectura.md     # Visión técnica de la memoria dual
│   │   └── vision_tecnica_v2.md # Evolución: Del mapa físico al lógico
│   ├── CONFIGURACION_MCP.md # Guía para configurar en Gemini/Claude
│   ├── MANUAL_DE_USO.md     # Manual para el usuario final
│   └── mcp_server.md        # Documentación técnica del servidor MCP
├── sql/               # Esquemas y migraciones
│   └── schema.sql     # Esquema principal SQLite + R-Tree (KAIROS)
├── src/               # Código fuente (Rust)
│   ├── main.rs        # Punto de entrada (Servidor MCP stdio)
│   ├── db.rs          # Gestor de persistencia SQLite
│   ├── embeddings.rs  # Motor de vectores (OpenAI Compatible)
│   ├── positional.rs  # Lógica de posicionamiento jerárquico 2D
│   ├── scanner.rs     # Escáner de archivos y extractor de metadatos
│   └── search.rs      # Motor de búsqueda híbrida semántica-espacial
├── kairos.db          # Base de datos SQLite generada (Vector Store)
├── .env               # Variables de entorno (API keys, URLs)
├── Cargo.toml         # Manifiesto de dependencias Rust
├── GEMINI.md          # ESTE ARCHIVO — Protocolo de integridad
├── PENDIENTES.md      # Roadmap detallado de tareas
└── conversacion.md    # Visión original del proyecto
```

---

## 📎 APÉNDICE: POR QUÉ EXISTE CADA REGLA

| Regla | Qué salió mal sin ella |
|-------|----------------------|
| 2.1 Prohibir código conceptual | Se generaron 86KB de diseños en `conversacion.md` que nunca se implementaron. El usuario creyó que estaban "listos". |
| 2.2 Prohibir promesas sin implementación | La IA dijo "tu sistema sería el primero en tener memoria espacial" — validando sin verificar viabilidad. |
| 2.3 Compilador como juez | Se acumularon 18 warnings de código muerto porque nadie ejecutó `cargo check` como gate. |
| 2.4 Las 3 preguntas | Se implementó `init_collection()` que nadie llama. Se creó `Config` que nadie instancia. Código muerto desde el nacimiento. |
| 2.5 Prohibir fallos silenciosos | Qdrant fallaba con `.ok()` y el usuario creía que todo indexaba correctamente. |
| 3.2 Incremento mínimo | Se intentó implementar Scanner + Qdrant + Search + MCP de golpe. Ninguno quedó completo. |
| 3.3 Protocolo/Especificación | Se implementó MCP sin leer la spec. No se soporta Content-Length framing ni notificaciones. |
