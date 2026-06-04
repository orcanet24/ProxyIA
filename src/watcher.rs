// src/watcher.rs
// Watch mode: re-indexación automática de archivos modificados en tiempo real.
// Usa notify::Watcher para detectar cambios en el filesystem.
// MEJORADO v2.2: Re-indexa automáticamente el archivo modificado.
//   - Rescanea solo el archivo cambiado (no todo el proyecto)
//   - Actualiza embedding + hash + summary
//   - Notifica al log el resultado

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use notify::{Watcher, RecursiveMode, Event, EventKind};
use tokio::sync::Mutex;

use crate::db::DatabaseManager;
use crate::scanner::FileScanner;

// Normaliza paths para consistencia cross-platform
fn norm(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    s.trim_start_matches("//?/").trim_start_matches("\\\\?\\").to_string()
}

/// Inicia el watcher en un hilo separado con re-indexación automática.
/// Recibe un FileScanner (con embedder + llm) para re-indexar archivos modificados.
/// Devuelve un WatcherGuard que mantiene el watcher vivo.
pub fn start_project_watcher(
    project_path: PathBuf,
    project_id: i64,
    db: DatabaseManager,
    scanner: Arc<Mutex<FileScanner>>,
) -> Result<impl std::ops::Drop> {
    let (std_tx, std_rx) = std::sync::mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
        if let Ok(event) = res {
            match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) => {
                    for path in event.paths {
                        if path.is_file() {
                            let _ = std_tx.send(path);
                        }
                    }
                }
                _ => {}
            }
        }
    })?;

    watcher.watch(&project_path, RecursiveMode::Recursive)?;

    // Hilo que procesa eventos en background
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        loop {
            match std_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(path) => {
                    eprintln!("🔄 Watch: cambio detectado en '{}'", path.display());
                    rt.block_on(async {
                        match rescan_single_file(&db, &scanner, project_id, &path).await {
                            Ok(true) => {
                                eprintln!("✅ Watch: '{}' re-indexado correctamente.", path.display());
                            }
                            Ok(false) => {
                                eprintln!("ℹ️ Watch: '{}' ignorado (no es un archivo de código fuente indexable).", path.display());
                            }
                            Err(e) => {
                                eprintln!("❌ Watch: error al re-indexar '{}': {}", path.display(), e);
                            }
                        }
                    });
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    Ok(WatcherGuard { _watcher: watcher })
}

/// Re-indexa un solo archivo modificado.
/// Devuelve true si el archivo fue re-indexado, false si se ignoró.
pub async fn rescan_single_file(
    db: &DatabaseManager,
    scanner: &Arc<Mutex<FileScanner>>,
    project_id: i64,
    file_path: &Path,
) -> Result<bool> {
    let path_str = norm(file_path);

    // Verificar que el archivo existe y tiene extensión reconocible
    if !file_path.exists() || !file_path.is_file() {
        return Ok(false);
    }

    // Detectar si es un lenguaje que podemos indexar
    let scanner_lock = scanner.lock().await;
    let lang = scanner_lock.detect_language(file_path);
    if lang.is_none() || lang.as_deref() == Some("unknown") {
        return Ok(false);
    }

    // Verificar que el archivo ya estaba indexado
    match db.get_node_by_path(&path_str).await {
        Ok(Some((_node_id, _))) => {
            // Ya existe → re-indexar
            drop(scanner_lock);
            reindex_file(db, scanner, project_id, file_path, &path_str).await?;
            Ok(true)
        }
        Ok(None) => {
            // Archivo nuevo → puede ser nuevo en el proyecto
            eprintln!("   ⚠️ Archivo '{}' no estaba en el índice. Ejecuta index_project para actualizarlo.", path_str);
            Ok(false)
        }
        Err(e) => {
            Err(anyhow::anyhow!("Error al verificar archivo '{}': {}", path_str, e))
        }
    }
}

/// Re-indexa un archivo actualizando su embedding, hash y summary.
async fn reindex_file(
    db: &DatabaseManager,
    scanner: &Arc<Mutex<FileScanner>>,
    _project_id: i64,
    file_path: &Path,
    path_str: &str,
) -> Result<()> {
    use sha2::{Sha256, Digest};
    use std::fs;

    let content = fs::read_to_string(file_path)?;
    let hash = format!("{:x}", Sha256::digest(content.as_bytes()));

    // Generar nuevo summary (estructural)
    let scanner_lock = scanner.lock().await;
    let summary = scanner_lock.distiller.skeletonize(file_path).ok();

    // Generar nuevo embedding
    let embedding = if let Some(embedder) = &scanner_lock.embedder {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            embedder.embed(trimmed).await.ok()
        } else { None }
    } else { None };

    drop(scanner_lock);

    // Re-generar posición (estable por hash del path)
    let dir_path = file_path.parent()
        .map(|p| norm(p))
        .unwrap_or_default();
    let depth = path_str.matches('/').count() as i32;
    let position = crate::positional::StablePositioner::calculate_base(
        &dir_path, path_str, depth, 0
    );

    // Actualizar en DB usando el método público de DatabaseManager
    db.update_node_content(
        path_str,
        &hash,
        embedding,
        summary,
        Some(position),
    ).await?;

    Ok(())
}

pub struct WatcherGuard {
    _watcher: notify::RecommendedWatcher,
}

impl std::ops::Drop for WatcherGuard {
    fn drop(&mut self) {
        eprintln!("🛑 Watch: watcher detenido");
    }
}