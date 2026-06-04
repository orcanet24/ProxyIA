// src/llm.rs
// Cliente LLM ligero para generar resúmenes conceptuales de archivos.
// Solo se usa durante la indexación inicial. Costo único de tokens.
// API compatible con OpenAI (funciona con OpenAI, Claude API, Ollama, LocalAI, etc.)

use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    api_url: String,
    api_key: String,
    model: String,
}

impl LlmClient {
    pub fn from_env() -> Option<Arc<Self>> {
        let api_url = std::env::var("LLM_API_URL").ok().filter(|s| !s.is_empty())?;
        let api_key = std::env::var("LLM_API_KEY").ok().filter(|s| !s.is_empty())?;
        let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string());
        Some(Arc::new(Self {
            http: reqwest::Client::new(),
            api_url,
            api_key,
            model,
        }))
    }

    /// Genera un resumen conceptual de un archivo.
    /// Prompt optimizado para mínimo consumo de tokens (~100-300 por archivo).
    pub async fn summarize_file(&self, path: &str, content: &str) -> Result<String> {
        // Truncar si es muy grande
        let max_chars = 12000;
        let safe_content = if content.len() > max_chars {
            let mut end = max_chars;
            while !content.is_char_boundary(end) { end -= 1; }
            format!("{}...\n[TRUNCADO: el original tiene {} caracteres]", 
                &content[..end], content.len())
        } else {
            content.to_string()
        };

        let prompt = format!(
            "Resume este archivo en 3-6 líneas:\n\
             - Propósito principal\n\
             - Principales funciones/clases/exports\n\
             - Dependencias clave (qué importa o usa)\n\n\
             ARCHIVO: {}\n\n{}",
            path, safe_content
        );

        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": "Eres un analizador de código. Responde solo con el resumen técnico, sin introducciones."},
                {"role": "user", "content": prompt}
            ],
            "max_tokens": 400,
            "temperature": 0.1
        });

        let response = self.http.post(&self.api_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("LLM API error {}: {}", status, text));
        }

        let json: Value = response.json().await?;
        let summary = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();

        if summary.is_empty() {
            Err(anyhow::anyhow!("LLM devolvió respuesta vacía"))
        } else {
            Ok(summary)
        }
    }
}
