# ProxyIA - Benchmark Report

**Fecha:** 2026-06-04 18:18:07
**Version ProxyIA:** release build 2026-06-04
**Embeddings:** HashEmbedder 384d (local, sin LLM)
**Proyecto:** [directorio de proyecto real]
**Archivos Rust indexados:** 48
**Lineas de codigo totales:** 3872

---

## 1. Indexacion Inicial (Cold Start)

Primera indexacion del proyecto desde cero. Incluye:
- Escaneo de archivos
- Generacion de resumenes estructurales (L1 skeletonizer)
- Creacion de embeddings semanticos (384d)
- Indice posicional (dependencias entre archivos)

| Metrica | Valor |
|---|---|
| Tiempo total | **98477 ms** (98.5 seg) |
| Archivos/segundo | 0.5 arch/s |
| Exit code |  |


## 2. Re-indexacion (L1 Cache Warm)

Segunda indexacion del mismo proyecto. Los resumenes L1 ya existen en DB,
solo se regeneran embeddings si cambiaron los archivos.

| Metrica | Valor |
|---|---|
| Tiempo total | **112855 ms** (112.9 seg) |
| Mejora vs cold | **-15% mas rapido** |

## 3. Busqueda Semantica

Busqueda por embeddings semanticos. Equivalente a buscar
conceptos en el proyecto sin leer archivos completos.

| Query | Tiempo (ms) | Tokens ahorrados |
|---|---|---|
| function definitions and structs | 437 ms | N/A |
| error handling and validation | 180 ms | N/A |
| module initialization | 182 ms | N/A |
| **Promedio** | **266 ms** | |

## 4. Mapa Estructural (L1 Summaries)

Obtiene los esqueletos/resumenes L1 de TODO el proyecto.
Equivalente a entender la arquitectura sin leer archivos.

| Metrica | Valor |
|---|---|
| Tiempo | **883 ms** |
| Exit code |  |

---

## Resultados Consolidados

| Prueba | Tiempo | Ahorro vs lectura completa |
|---|---|---|
| Indexacion cold | 98477 ms | Indexa TODO el proyecto (1 vez) |
| Re-indexacion | 112855 ms (-15% mejora) | Usa cache L1 |
| Busqueda semantica | 266 ms (promedio) | **~99% tokens ahorrados** |
| Mapa estructural | 883 ms | **~100% tokens ahorrados** |

### Estimacion de Ahorro de Tokens

| Escenario | Tokens aprox | Equivalente |
|---|---|---|
| Leer todos los archivos completos | **~70.3125K tokens** | ~18 paginas de contexto |
| Busqueda semantica (ProxyIA) | **~500-2000 tokens** | Solo fragmentos relevantes |
| Mapa estructural (ProxyIA) | **~100-500 tokens** | Solo esqueletos L1 |
| **Ahorro estimado por consulta** | **~99%** | vs lectura completa |

### Notas

- Los tiempos incluyen arranque del proceso y carga de DB (HashEmbedder 384d)
- Embeddings locales: sin dependencias externas, sin llamadas HTTP
- Sin LLM: los resumenes L1 son estructurales (skeletonizer)
- Si se activa un LLM, los resumenes serian conceptuales (~10-30x mas lentos en indexacion)
- Proyecto indexado: proyecto real con 48+ archivos Rust
