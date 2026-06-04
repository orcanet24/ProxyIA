// src/db.rs
use sqlx::{sqlite::SqlitePool, Pool, Sqlite, Row};
use anyhow::Result;
use crate::positional::Position3D;
use std::sync::{Arc, RwLock};
use std::collections::HashMap;

// Límites de caché para control de memoria
const MAX_VECTOR_CACHE_ENTRIES: usize = 5;   // Máximo 5 proyectos en caché de vectores
const MAX_SUMMARY_CACHE_ENTRIES: usize = 10;  // Máximo 10 proyectos en caché de summaries

#[derive(Clone)]
pub struct DatabaseManager {
    pool: Pool<Sqlite>,
    vector_cache: Arc<RwLock<HashMap<i64, (Vec<(String, Vec<f32>)>, usize)>>>, // (datos, hits)
    summary_cache: Arc<RwLock<HashMap<i64, (Vec<(String, String)>, usize)>>>,   // (datos, hits)
}

impl DatabaseManager {
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = SqlitePool::connect(database_url).await?;
        let schema = include_str!("../sql/schema.sql");
        sqlx::query(schema).execute(&pool).await?;
        Ok(Self { 
            pool, 
            vector_cache: Arc::new(RwLock::new(HashMap::new())),
            summary_cache: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub async fn create_project(&self, name: &str, root_path: &str) -> Result<i64> {
        let sql = r#"
            INSERT INTO projects (name, root_path)
            VALUES (?, ?)
            ON CONFLICT(root_path) DO UPDATE SET last_accessed = strftime('%s', 'now')
            RETURNING id
        "#;
        let row = sqlx::query(sql)
            .bind(name).bind(root_path)
            .fetch_one(&self.pool).await?;
        Ok(row.get(0))
    }

    pub async fn get_project_id(&self, path: &str) -> Result<Option<i64>> {
        // Primero intenta match exacto
        let row = sqlx::query("SELECT id FROM projects WHERE root_path = ?")
            .bind(path).fetch_optional(&self.pool).await?;
        if let Some(r) = row {
            return Ok(Some(r.get(0)));
        }
        // Fallback: match por prefijo (el path actual puede ser un subdirectorio del proyecto indexado)
        let rows = sqlx::query("SELECT id, root_path FROM projects ORDER BY LENGTH(root_path) DESC")
            .fetch_all(&self.pool).await?;
        for r in rows {
            let root_path: String = r.get(1);
            if path.starts_with(&root_path) || root_path.starts_with(path) {
                return Ok(Some(r.get(0)));
            }
        }
        // Último recurso: el proyecto más recientemente accedido
        let last = sqlx::query("SELECT id FROM projects ORDER BY last_accessed DESC LIMIT 1")
            .fetch_optional(&self.pool).await?;
        Ok(last.map(|r| r.get(0)))
    }

    pub async fn list_all_projects(&self) -> Result<Vec<(String, String)>> {
        let rows = sqlx::query("SELECT name, root_path FROM projects")
            .fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    #[allow(dead_code)]
    pub async fn delete_project_by_path(&self, path: &str) -> Result<()> {
        sqlx::query("DELETE FROM projects WHERE root_path = ?")
            .bind(path).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_neighbors_by_id(&self, node_id: i64, limit: i64) -> Result<Vec<(String, Position3D)>> {
        // Usar fetch_optional para evitar error si no existe en el índice espacial
        let row = sqlx::query("SELECT min_x, min_y FROM filesystem_spatial_idx WHERE id = ?")
            .bind(node_id).fetch_optional(&self.pool).await?;

        let (x, y) = match row {
            Some(r) => (r.get::<f32, _>(0), r.get::<f32, _>(1)),
            None => return Ok(Vec::new()), // Sin posición espacial → sin vecinos
        };

        let radius = 0.5;

        let rows = sqlx::query(r#"
            SELECT f.path, f.position_vector
            FROM filesystem_tree f
            JOIN filesystem_spatial_idx s ON f.id = s.id
            WHERE s.min_x BETWEEN ? AND ? AND s.min_y BETWEEN ? AND ?
            AND f.id != ?
            LIMIT ?
        "#)
        .bind(x - radius).bind(x + radius)
        .bind(y - radius).bind(y + radius)
        .bind(node_id)
        .bind(limit)
        .fetch_all(&self.pool).await?;

        let mut results = Vec::new();
        for r in rows {
            let path: String = r.get(0);
            let bytes: Vec<u8> = r.get(1);
            if let Some(pos) = Position3D::from_bytes(&bytes) {
                results.push((path, pos));
            }
        }
        Ok(results)
    }

    pub async fn insert_node(&self, project_id: i64, name: &str, path: &str, parent_id: Option<i64>, depth: i32, position: Position3D, node_type: &str, language: Option<&str>, semantic_vector: Option<Vec<f32>>, content_hash: Option<&str>, structural_summary: Option<&str>) -> Result<i64> {
        let pos_bytes = position.to_bytes();
        let sem_bytes = semantic_vector.as_ref().map(|v| {
            let mut bytes = Vec::with_capacity(v.len() * 4);
            for f in v { bytes.extend_from_slice(&f.to_le_bytes()); }
            bytes
        });

        let sql = r#"
            INSERT INTO filesystem_tree (project_id, name, path, parent_id, depth, position_vector, node_type, language, semantic_vector, content_hash, structural_summary)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(path) DO UPDATE SET
                name = excluded.name,
                project_id = excluded.project_id,
                parent_id = excluded.parent_id,
                depth = excluded.depth,
                position_vector = excluded.position_vector,
                node_type = excluded.node_type,
                language = excluded.language,
                semantic_vector = COALESCE(excluded.semantic_vector, filesystem_tree.semantic_vector),
                content_hash = COALESCE(excluded.content_hash, filesystem_tree.content_hash),
                structural_summary = COALESCE(excluded.structural_summary, filesystem_tree.structural_summary),
                last_indexed = strftime('%s', 'now')
            RETURNING id
        "#;
        let row = sqlx::query(sql)
            .bind(project_id).bind(name).bind(path).bind(parent_id).bind(depth).bind(pos_bytes).bind(node_type).bind(language).bind(sem_bytes).bind(content_hash).bind(structural_summary)
            .fetch_one(&self.pool).await?;
        
        let node_id: i64 = row.get(0);

        let rtree_sql = r#"
            INSERT OR REPLACE INTO filesystem_spatial_idx (id, min_x, max_x, min_y, max_y, min_z, max_z)
            VALUES (?, ?, ?, ?, ?, ?, ?)
        "#;
        sqlx::query(rtree_sql)
            .bind(node_id)
            .bind(position.x).bind(position.x)
            .bind(position.y).bind(position.y)
            .bind(position.z).bind(position.z)
            .execute(&self.pool).await?;

        // Invalidar cachés
        if semantic_vector.is_some() {
            if let Ok(mut cache) = self.vector_cache.write() { cache.remove(&project_id); }
        }
        if structural_summary.is_some() {
            if let Ok(mut cache) = self.summary_cache.write() { cache.remove(&project_id); }
        }

        Ok(node_id)
    }

    pub async fn get_node_hash(&self, path: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT content_hash FROM filesystem_tree WHERE path = ?")
            .bind(path).fetch_optional(&self.pool).await?;
        Ok(row.and_then(|r| r.get(0)))
    }

    pub async fn insert_dependency(&self, source_id: i64, target_id: i64, dep_type: &str) -> Result<()> {     
        sqlx::query("INSERT OR IGNORE INTO dependencies (source_id, target_id, dependency_type) VALUES (?, ?, ?)")
            .bind(source_id).bind(target_id).bind(dep_type)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn get_node_by_path(&self, path: &str) -> Result<Option<(i64, Vec<u8>)>> {
        let row = sqlx::query("SELECT id, position_vector FROM filesystem_tree WHERE path = ?")
            .bind(path).fetch_optional(&self.pool).await?;
        Ok(row.map(|r| (r.get(0), r.get(1))))
    }

    pub async fn get_node_summary(&self, path: &str) -> Result<Option<String>> {
        let row = sqlx::query("SELECT structural_summary FROM filesystem_tree WHERE path = ? AND structural_summary IS NOT NULL")
            .bind(path).fetch_optional(&self.pool).await?;
        Ok(row.and_then(|r| r.get(0)))
    }

    pub async fn delete_nodes_not_on_disk(&self, project_id: i64, existing_paths: &[String]) -> Result<usize> {
        // Obtener todos los paths indexados para este proyecto
        let rows = sqlx::query("SELECT id, path FROM filesystem_tree WHERE project_id = ? AND node_type = 'file'")
            .bind(project_id).fetch_all(&self.pool).await?;

        let mut deleted_count = 0usize;
        for row in rows {
            let node_id: i64 = row.get(0);
            let indexed_path: String = row.get(1);
            if !existing_paths.contains(&indexed_path) {
                // Eliminar del R-Tree espacial
                sqlx::query("DELETE FROM filesystem_spatial_idx WHERE id = ?")
                    .bind(node_id).execute(&self.pool).await?;
                // Eliminar dependencias
                sqlx::query("DELETE FROM dependencies WHERE source_id = ? OR target_id = ?")
                    .bind(node_id).bind(node_id).execute(&self.pool).await?;
                // Eliminar del árbol principal
                sqlx::query("DELETE FROM filesystem_tree WHERE id = ?")
                    .bind(node_id).execute(&self.pool).await?;
                deleted_count += 1;
            }
        }

        // Invalidar cachés
        {
            if let Ok(mut cache) = self.vector_cache.write() { cache.remove(&project_id); }
            if let Ok(mut cache) = self.summary_cache.write() { cache.remove(&project_id); }
        }

        Ok(deleted_count)
    }

    pub async fn get_all_nodes(&self, project_id: i64) -> Result<Vec<(i64, String, Vec<u8>)>> {
        let rows = sqlx::query("SELECT id, path, position_vector FROM filesystem_tree WHERE project_id = ?")
            .bind(project_id).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1), r.get(2))).collect())
    }

    pub async fn get_positional_neighbors(&self, x: f32, y: f32, z: f32, radius: f32, exclude_id: Option<i64>) -> Result<Vec<i64>> {
        let min_x = x - radius;
        let max_x = x + radius;
        let min_y = y - radius;
        let max_y = y + radius;
        let min_z = (z - radius).max(0.0);
        let max_z = z + radius;

        let rows = sqlx::query(
            "SELECT id FROM filesystem_spatial_idx WHERE min_x >= ? AND max_x <= ? AND min_y >= ? AND max_y <= ? AND min_z >= ? AND max_z <= ?"
        )
            .bind(min_x).bind(max_x).bind(min_y).bind(max_y).bind(min_z).bind(max_z)
            .fetch_all(&self.pool).await?;

        let ids: Vec<i64> = rows.into_iter().map(|r| r.get(0)).collect();

        // Excluir el nodo consultante si está en los resultados
        if let Some(exclude) = exclude_id {
            Ok(ids.into_iter().filter(|id| *id != exclude).collect())
        } else {
            Ok(ids)
        }
    }

    /// Inserta en caché con política LRU: si se excede el límite, elimina la entrada con menos hits.
    fn insert_vector_cache(&self, cache: &mut HashMap<i64, (Vec<(String, Vec<f32>)>, usize)>, pid: i64, data: Vec<(String, Vec<f32>)>) {
        if cache.len() >= MAX_VECTOR_CACHE_ENTRIES && !cache.contains_key(&pid) {
            // LRU: eliminar la entrada con menos hits
            let min_key = cache.iter()
                .min_by_key(|entry| entry.1 .1) // (_, (_, hits)) → hits es .1.1
                .map(|(k, _)| *k);
            if let Some(key) = min_key {
                cache.remove(&key);
            }
        }
        cache.insert(pid, (data, 0));
    }

    /// Inserta en caché de summaries con política LRU
    fn insert_summary_cache(&self, cache: &mut HashMap<i64, (Vec<(String, String)>, usize)>, pid: i64, data: Vec<(String, String)>) {
        if cache.len() >= MAX_SUMMARY_CACHE_ENTRIES && !cache.contains_key(&pid) {
            let min_key = cache.iter()
                .min_by_key(|entry| entry.1 .1) // (_, (_, hits)) → hits es .1.1
                .map(|(k, _)| *k);
            if let Some(key) = min_key {
                cache.remove(&key);
            }
        }
        cache.insert(pid, (data, 0));
    }

    pub async fn get_all_vectors(&self, project_id: i64) -> Result<Vec<(String, Vec<f32>)>> {
        // Revisar caché con tracking de hits
        {
            if let Ok(mut cache) = self.vector_cache.write() {
                if let Some((data, hits)) = cache.get_mut(&project_id) {
                    *hits += 1; // Incrementar contador de uso
                    return Ok(data.clone());
                }
            }
        }

        let rows = sqlx::query("SELECT path, semantic_vector FROM filesystem_tree WHERE project_id = ? AND semantic_vector IS NOT NULL")
            .bind(project_id).fetch_all(&self.pool).await?;

        let mut results = Vec::new();
        for row in rows {
            let path: String = row.get(0);
            let blob: Vec<u8> = row.get(1);
            let mut vector = Vec::with_capacity(blob.len() / 4);
            for chunk in blob.chunks_exact(4) {
                let bytes: [u8; 4] = chunk.try_into().unwrap();
                vector.push(f32::from_le_bytes(bytes));
            }
            results.push((path, vector));
        }

        // Almacenar en caché con LRU
        {
            if let Ok(mut cache) = self.vector_cache.write() {
                self.insert_vector_cache(&mut cache, project_id, results.clone());
            }
        }

        Ok(results)
    }

    pub async fn get_project_summaries(&self, project_id: i64) -> Result<Vec<(String, String)>> {
        // Revisar caché con tracking de hits
        {
            if let Ok(mut cache) = self.summary_cache.write() {
                if let Some((data, hits)) = cache.get_mut(&project_id) {
                    *hits += 1;
                    return Ok(data.clone());
                }
            }
        }

        let rows = sqlx::query("SELECT path, structural_summary FROM filesystem_tree WHERE project_id = ? AND structural_summary IS NOT NULL")
            .bind(project_id).fetch_all(&self.pool).await?;

        let results: Vec<(String, String)> = rows.into_iter().map(|r| (r.get(0), r.get(1))).collect();

        // Almacenar en caché con LRU
        {
            if let Ok(mut cache) = self.summary_cache.write() {
                self.insert_summary_cache(&mut cache, project_id, results.clone());
            }
        }

        Ok(results)
    }

    pub async fn get_project_dependencies(&self, project_id: i64) -> Result<Vec<(i64, i64)>> {
        let rows = sqlx::query(r#"
            SELECT d.source_id, d.target_id
            FROM dependencies d
            JOIN filesystem_tree f ON d.source_id = f.id
            WHERE f.project_id = ?
        "#)
        .bind(project_id)
        .fetch_all(&self.pool).await?;

        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    pub async fn update_node_position(&self, id: i64, position: Position3D) -> Result<()> {
        let pos_bytes = position.to_bytes();

        sqlx::query("UPDATE filesystem_tree SET position_vector = ? WHERE id = ?")
            .bind(pos_bytes).bind(id)
            .execute(&self.pool).await?;

        sqlx::query("UPDATE filesystem_spatial_idx SET min_x = ?, max_x = ?, min_y = ?, max_y = ?, min_z = ?, max_z = ? WHERE id = ?")
            .bind(position.x).bind(position.x)
            .bind(position.y).bind(position.y)
            .bind(position.z).bind(position.z)
            .bind(id)
            .execute(&self.pool).await?;

        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_node_by_id(&self, id: i64) -> Result<Option<(String, Position3D)>> {
        let row = sqlx::query("SELECT path, position_vector FROM filesystem_tree WHERE id = ?")
            .bind(id).fetch_optional(&self.pool).await?;

        if let Some(r) = row {
            let path: String = r.get(0);
            let bytes: Vec<u8> = r.get(1);
            if let Some(pos) = Position3D::from_bytes(&bytes) {
                return Ok(Some((path, pos)));
            }
        }
        Ok(None)
    }

    // ── Funciones individuales ─────────────────────────────────────────

    /// Inserta o actualiza una función individual en function_index
    pub async fn insert_function(
        &self,
        node_id: i64,
        project_id: i64,
        name: &str,
        signature: &str,
        start_line: i32,
        end_line: i32,
        file_path: &str,
        func_type: &str,
        is_public: bool,
        semantic_vector: Option<Vec<f32>>,
        content_hash: Option<&str>,
    ) -> Result<i64> {
        let sem_bytes = semantic_vector.as_ref().map(|v| {
            let mut bytes = Vec::with_capacity(v.len() * 4);
            for f in v { bytes.extend_from_slice(&f.to_le_bytes()); }
            bytes
        });

        let sql = r#"
            INSERT INTO function_index (node_id, project_id, name, signature, start_line, end_line,
                                        file_path, func_type, is_public, semantic_vector, content_hash)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(node_id, name) DO UPDATE SET
                signature = excluded.signature,
                start_line = excluded.start_line,
                end_line = excluded.end_line,
                file_path = excluded.file_path,
                func_type = excluded.func_type,
                is_public = excluded.is_public,
                semantic_vector = COALESCE(excluded.semantic_vector, function_index.semantic_vector),
                content_hash = COALESCE(excluded.content_hash, function_index.content_hash)
            RETURNING id
        "#;

        let row = sqlx::query(sql)
            .bind(node_id).bind(project_id).bind(name).bind(signature)
            .bind(start_line).bind(end_line).bind(file_path)
            .bind(func_type).bind(is_public)
            .bind(sem_bytes).bind(content_hash)
            .fetch_one(&self.pool).await?;

        Ok(row.get(0))
    }

    /// Busca funciones por nombre (coincidencia exacta o LIKE)
    pub async fn search_functions_by_name(&self, project_id: i64, name_query: &str, limit: usize) -> Result<Vec<(i64, String, String, String, i32, i32, f32)>> {
        let pattern = format!("%{}%", name_query);
        let rows = sqlx::query(
            "SELECT id, name, signature, file_path, start_line, end_line FROM function_index \
             WHERE project_id = ? AND name LIKE ? LIMIT ?"
        )
            .bind(project_id).bind(&pattern).bind(limit as i64)
            .fetch_all(&self.pool).await?;

        Ok(rows.into_iter().map(|r| {
            let id: i64 = r.get(0);
            let name: String = r.get(1);
            let signature: String = r.get(2);
            let file_path: String = r.get(3);
            let start_line: i32 = r.get(4);
            let end_line: i32 = r.get(5);
            (id, name, signature, file_path, start_line, end_line, 1.0)
        }).collect())
    }

    /// Obtiene todas las funciones indexadas para un proyecto
    pub async fn get_all_functions(&self, project_id: i64) -> Result<Vec<(i64, String, String, String, i32, i32, Vec<u8>)>> {
        let rows = sqlx::query(
            "SELECT id, name, signature, file_path, start_line, end_line, semantic_vector \
             FROM function_index WHERE project_id = ? AND semantic_vector IS NOT NULL"
        )
            .bind(project_id)
            .fetch_all(&self.pool).await?;

        Ok(rows.into_iter().map(|r| {
            (r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5), r.get(6))
        }).collect())
    }

    /// Busca funciones por similitud semántica (coseno) sobre sus embeddings
    pub async fn search_functions_semantic(
        &self,
        project_id: i64,
        query_vector: &[f32],
        top_k: usize,
    ) -> Result<Vec<(String, String, String, i32, i32, f32)>> {
        let all_funcs = self.get_all_functions(project_id).await?;
        let mut results = Vec::new();

        for (_, name, sig, path, start, end, blob) in all_funcs {
            if blob.len() < 4 { continue; }
            let mut vector = Vec::with_capacity(blob.len() / 4);
            for chunk in blob.chunks_exact(4) {
                let bytes: [u8; 4] = chunk.try_into().unwrap();
                vector.push(f32::from_le_bytes(bytes));
            }
            let score = cosine_similarity(query_vector, &vector);
            results.push((name, sig, path, start, end, score));
        }

        results.sort_by(|a, b| b.5.partial_cmp(&a.5).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k);
        Ok(results)
    }

    /// Inserta dependencia entre funciones
    pub async fn insert_function_dependency(&self, source_func_id: i64, target_func_id: i64, dep_type: &str) -> Result<()> {
        sqlx::query("INSERT OR IGNORE INTO function_dependencies (source_func_id, target_func_id, dep_type) VALUES (?, ?, ?)")
            .bind(source_func_id).bind(target_func_id).bind(dep_type)
            .execute(&self.pool).await?;
        Ok(())
    }

    /// Obtiene las dependencias entre funciones de un proyecto
    pub async fn get_function_dependencies(&self, project_id: i64) -> Result<Vec<(i64, i64)>> {
        let rows = sqlx::query(
            "SELECT fd.source_func_id, fd.target_func_id \
             FROM function_dependencies fd \
             JOIN function_index f ON fd.source_func_id = f.id \
             WHERE f.project_id = ?"
        )
            .bind(project_id)
            .fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
    }

    /// Obtiene el número de funciones públicas en un archivo (para Z)
    pub async fn count_public_functions_in_file(&self, node_id: i64) -> Result<i32> {
        let row = sqlx::query("SELECT COUNT(*) FROM function_index WHERE node_id = ? AND is_public = 1")
            .bind(node_id)
            .fetch_one(&self.pool).await?;
        let count: i64 = row.get(0);
        Ok(count as i32)
    }

    /// Actualiza el contenido de un nodo existente (para re-indexación en watch mode)
    pub async fn update_node_content(
        &self,
        path: &str,
        content_hash: &str,
        semantic_vector: Option<Vec<f32>>,
        structural_summary: Option<String>,
        position: Option<Position3D>,
    ) -> Result<()> {
        let sem_bytes = semantic_vector.as_ref().map(|v| {
            let mut bytes = Vec::with_capacity(v.len() * 4);
            for f in v { bytes.extend_from_slice(&f.to_le_bytes()); }
            bytes
        });
        let pos_bytes = position.as_ref().map(|p| p.to_bytes());

        sqlx::query(
            "UPDATE filesystem_tree SET \
             content_hash = ?, \
             semantic_vector = ?, \
             structural_summary = ?, \
             position_vector = COALESCE(?, position_vector), \
             last_indexed = strftime('%s', 'now') \
             WHERE path = ?"
        )
        .bind(content_hash)
        .bind(sem_bytes)
        .bind(structural_summary)
        .bind(pos_bytes)
        .bind(path)
        .execute(&self.pool).await?;

        Ok(())
    }

    pub async fn query_similar_summaries(
        &self,
        project_id: i64,
        query_vector: &[f32],
        threshold: f32,
        top_k: usize,
    ) -> Result<Vec<(String, String, f32)>> {
        let all_vectors = self.get_all_vectors(project_id).await?;
        let summaries = self.get_project_summaries(project_id).await?;
        
        let mut summary_map: HashMap<String, String> = summaries.into_iter().collect();
        let mut results = Vec::new();
        
        for (path, vector) in all_vectors {
            let score = cosine_similarity(query_vector, &vector);
            if score >= threshold {
                if let Some(summary) = summary_map.remove(&path) {
                    results.push((path, summary, score));
                }
            }
        }
        
        results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k);
        
        Ok(results)
    }
}

fn cosine_similarity(v1: &[f32], v2: &[f32]) -> f32 {
    let dot_product: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
    let norm_v1: f32 = v1.iter().map(|a| a * a).sum::<f32>().sqrt();
    let norm_v2: f32 = v2.iter().map(|a| a * a).sum::<f32>().sqrt();
    if norm_v1 == 0.0 || norm_v2 == 0.0 {
        0.0
    } else {
        dot_product / (norm_v1 * norm_v2)
    }
}
