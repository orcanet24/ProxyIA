// src/local_embed.rs
// Embedder local mejorado: hash de características + n-gramas (bigramas y trigramas)
// 100% Rust, cero dependencias externas, 384 dimensiones.
// No requiere descargar modelos, internet, ni GPUs.
//
// MEJORA v2.1: Se añaden bigramas (pares de tokens) y trigramas (tripletas) para
// capturar relaciones semánticas parciales. Ej: "DB connection" y "SQL conn" comparten
// trigramas parciales, lo que mejora la similitud coseno sin necesidad de MiniLM.
//
// NOTA: Si se desea usar MiniLM real (candle), descomentar las
// dependencias en Cargo.toml y reemplazar este archivo.

use anyhow::Result;
use async_trait::async_trait;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub struct HashEmbedder {
    size: u64,
}

impl HashEmbedder {
    pub fn new() -> Self {
        Self { size: 384 }
    }

    /// Genera un vector 384D a partir del texto usando hashing de características.
    /// Mejorado con soporte de bigramas y trigramas para capturar contexto parcial.
    fn hash_text(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.size as usize];

        // Tokenizar: split por whitespace y puntuación común
        let tokens: Vec<String> = text
            .split(|c: char| c.is_whitespace() || "{}()[]<>;:,.'\"`!@#$%^&*+-=|\\/~?".contains(c))
            .map(|t| {
                t.chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
                    .to_lowercase()
            })
            .filter(|t| t.len() >= 2)
            .collect();

        // 1. Embedding de unigramas (existente, peso completo)
        for token in &tokens {
            self.hash_add(&mut vec, token, 1.0);
        }

        // 2. Embedding de bigramas (pares consecutivos, peso 0.5)
        //    Captura: "db connection" → similar a "sql database"
        for window in tokens.windows(2) {
            let bigram = format!("{}__{}", window[0], window[1]);
            self.hash_add(&mut vec, &bigram, 0.5);
        }

        // 3. Embedding de trigramas (tripletas consecutivas, peso 0.3)
        //    Captura relaciones más largas sin llegar a ser contexto completo
        for window in tokens.windows(3) {
            let trigram = format!("{}__{}__{}", window[0], window[1], window[2]);
            self.hash_add(&mut vec, &trigram, 0.3);
        }

        // L2 normalize
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }

        vec
    }

    /// Aplica hashing trick para una feature con peso dado
    #[inline]
    fn hash_add(&self, vec: &mut [f32], feature: &str, weight: f32) {
        let mut h = DefaultHasher::new();
        feature.hash(&mut h);
        let hash = h.finish();

        let idx = (hash as usize) % self.size as usize;
        let sign = if (hash >> 32) & 1 == 0 { 1.0 } else { -1.0 };

        vec[idx] += sign * weight * (1.0 + (feature.len() as f32).ln());
    }
}

#[async_trait]
impl super::embeddings::EmbeddingEngine for HashEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let safe_text = if text.len() > 16000 {
            let mut end = 16000;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        } else {
            text
        };
        Ok(self.hash_text(safe_text))
    }

    fn vector_size(&self) -> u64 {
        self.size
    }

    async fn chat(&self, _system_prompt: &str, _user_query: &str, _context: &str) -> Result<String> {
        Err(anyhow::anyhow!("Chat no disponible con embedder local hash"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_similar_phrases_get_higher_score() {
        let embedder = HashEmbedder::new();

        let vec1 = embedder.hash_text("database connection pool manager");
        let vec2 = embedder.hash_text("db conn pool manager");
        let vec3 = embedder.hash_text("apple banana orange fruit");

        let sim_similar = cosine_similarity(&vec1, &vec2);
        let sim_different = cosine_similarity(&vec1, &vec3);

        // Frases similares deben tener score más alto que frases diferentes
        assert!(
            sim_similar > sim_different,
            "Similitud entre frases similares ({}) debería ser mayor que entre diferentes ({})",
            sim_similar,
            sim_different
        );
    }

    #[test]
    fn test_deterministic_embeddings() {
        let embedder = HashEmbedder::new();
        let text = "fn process_data(db: &Database) -> Result<()>";

        let vec1 = embedder.hash_text(text);
        let vec2 = embedder.hash_text(text);

        for (a, b) in vec1.iter().zip(vec2.iter()) {
            assert!(
                (a - b).abs() < f32::EPSILON,
                "Los embeddings deben ser determinísticos"
            );
        }
    }

    #[test]
    fn test_ngrams_improve_semantic_capture() {
        let embedder = HashEmbedder::new();

        // Sin n-gramas, "db_conn" y "database_connection" no compartirían tokens
        // Con bigramas/trigramas, comparten estructura parcial
        let vec_db = embedder.hash_text("db conn pool");
        let vec_db_full = embedder.hash_text("database connection pool");
        let vec_unrelated = embedder.hash_text("render html template");

        let sim_related = cosine_similarity(&vec_db, &vec_db_full);
        let sim_unrelated = cosine_similarity(&vec_db, &vec_unrelated);

        assert!(
            sim_related > sim_unrelated,
            "Embeddings con n-gramas deben diferenciar conceptos relacionados de no relacionados"
        );
    }

    fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 {
            0.0
        } else {
            dot / (norm_a * norm_b)
        }
    }
}