// src/search.rs
use crate::positional::Position3D;
use crate::db::DatabaseManager;
use crate::embeddings::EmbeddingEngine;
use anyhow::Result;
use std::sync::Arc;
use std::sync::Mutex;
use std::collections::HashMap;
use std::sync::LazyLock;

// Caché global de vectores de consulta (LRU simple, máx 32 entradas)
static QUERY_CACHE: LazyLock<Mutex<HashMap<String, Vec<f32>>>> = LazyLock::new(|| {
    Mutex::new(HashMap::new())
});
const MAX_CACHE_ENTRIES: usize = 32;

pub struct HybridSearcher {
    db: DatabaseManager,
    embedder: Arc<dyn EmbeddingEngine>,
}

impl HybridSearcher {
    pub fn new(db: DatabaseManager, embedder: Arc<dyn EmbeddingEngine>) -> Self {
        Self { db, embedder }
    }

    async fn get_or_embed(&self, query: &str) -> Result<Vec<f32>> {
        // Revisar caché
        {
            let cache = QUERY_CACHE.lock().unwrap();
            if let Some(vec) = cache.get(query) {
                return Ok(vec.clone());
            }
        }
        // Embedear
        let vec = self.embedder.embed(query).await?;
        // Almacenar en caché
        {
            let mut cache = QUERY_CACHE.lock().unwrap();
            if cache.len() >= MAX_CACHE_ENTRIES {
                cache.clear(); // LRU simple: limpiar todo
            }
            cache.insert(query.to_string(), vec.clone());
        }
        Ok(vec)
    }

    pub async fn search(&self, project_id: i64, query: &str, limit: usize) -> Result<Vec<SearchResult>> {
        let query_vector = self.get_or_embed(query).await?;
        let all_vectors = self.db.get_all_vectors(project_id).await?;

        if all_vectors.is_empty() {
            return Err(anyhow::anyhow!(
                "No hay vectores semánticos en este proyecto. Ejecuta 'index_project' primero."
            ));
        }

        let mut scored_results: Vec<(String, f32)> = all_vectors.into_iter()
            .map(|(path, v)| (path, Self::cosine_similarity(&query_vector, &v)))
            .collect();

        // Ordenar por similitud (mayor a menor)
        scored_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let top_score = scored_results.first().map(|(_, s)| *s).unwrap_or(1.0);

        let top_semantic = scored_results.into_iter().take(limit * 2).collect::<Vec<_>>();

        let mut results = Vec::new();
        for (path, semantic_score) in top_semantic {
            if let Some((node_id, pos_bytes)) = self.db.get_node_by_path(&path).await? {
                let pos = match Position3D::from_bytes(&pos_bytes) {
                    Some(p) => p,
                    None => {
                        eprintln!("⚠️ Posición inválida en DB para '{}', saltando.", path);
                        continue;
                    }
                };

                // Scoring posicional 3D: cluster bonus basado en densidad vecinal
                let neighbors = self.db.get_positional_neighbors(pos.x, pos.y, pos.z, 0.2, Some(node_id)).await.unwrap_or_default();
                let density_bonus = if neighbors.len() > 5 {
                    0.30 // Cluster denso
                } else if neighbors.len() > 2 {
                    0.15 // Cluster medio
                } else {
                    0.0 // Aislado
                };

                // Obtener el resumen estructural si existe
                let summary = self.db.get_node_summary(&path).await.unwrap_or(None);

                results.push(SearchResult {
                    path,
                    semantic_score,
                    positional_score: density_bonus,
                    combined_score: (semantic_score / top_score.max(0.01)) * 0.7 + density_bonus,
                    summary,
                });
            }
        }

        results.sort_by(|a, b| b.combined_score.partial_cmp(&a.combined_score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);

        Ok(results)
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() { return 0.0; }
        let mut dot = 0.0;
        let mut norm_a = 0.0;
        let mut norm_b = 0.0;
        for i in 0..a.len() {
            dot += a[i] * b[i];
            norm_a += a[i] * a[i];
            norm_b += b[i] * b[i];
        }
        if norm_a == 0.0 || norm_b == 0.0 { return 0.0; }
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

pub struct SearchResult {
    pub path: String,
    pub semantic_score: f32,
    pub positional_score: f32,
    pub combined_score: f32,
    pub summary: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_embed::HashEmbedder;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = HybridSearcher::cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 0.0001, "Expected 1.0, got {}", sim);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = HybridSearcher::cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 0.0001, "Expected 0.0, got {}", sim);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        let a: Vec<f32> = vec![];
        let b: Vec<f32> = vec![];
        let sim = HybridSearcher::cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 0.0001, "Expected 0.0, got {}", sim);
    }

    #[test]
    fn test_cosine_similarity_different_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let sim = HybridSearcher::cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 0.0001, "Expected 0.0, got {}", sim);
    }

    #[test]
    fn test_hybrid_search_result_structure() {
        let result = SearchResult {
            path: "/test/file.rs".to_string(),
            semantic_score: 0.85,
            positional_score: 0.15,
            combined_score: 0.75,
            summary: Some("fn test()".to_string()),
        };
        assert_eq!(result.path, "/test/file.rs");
        assert!((result.semantic_score - 0.85).abs() < 0.0001);
        assert!((result.combined_score - 0.75).abs() < 0.0001);
        assert_eq!(result.summary.unwrap(), "fn test()");
    }

    #[test]
    fn test_hash_embedder_search_consistency() {
        let embedder = HashEmbedder::new();
        let query = "test query for search";
        // Usamos runtime tokio para llamar embed (async)
        let rt = tokio::runtime::Runtime::new().unwrap();
        let v1 = rt.block_on(embedder.embed(query)).unwrap();
        let v2 = rt.block_on(embedder.embed(query)).unwrap();
        // Misma query → mismo vector
        assert_eq!(v1.len(), v2.len());
        let sim = HybridSearcher::cosine_similarity(&v1, &v2);
        assert!((sim - 1.0).abs() < 0.0001, "Expected deterministic embeddings, got sim={}", sim);
    }
}
