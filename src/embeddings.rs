// src/embeddings.rs
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use reqwest::Client;

#[async_trait]
pub trait EmbeddingEngine: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    #[allow(dead_code)]
    fn vector_size(&self) -> u64;
    #[allow(dead_code)]
    async fn chat(&self, system_prompt: &str, user_query: &str, context: &str) -> Result<String>;
}

pub struct OpenAIEmbedder {
    client: Client,
    api_key: String,
    url: String,
    #[allow(dead_code)]
    chat_url: String,
    embedding_model: String,
    #[allow(dead_code)]
    chat_model: String,
    #[allow(dead_code)]
    size: u64,
}

#[allow(dead_code)]
impl OpenAIEmbedder {
    pub fn new(api_key: String, url: String, embedding_model: String, chat_model: String, size: u64) -> Self {
        let chat_url = url.replace("/embeddings", "/chat/completions");
        Self { client: Client::new(), api_key, url, chat_url, embedding_model, chat_model, size }
    }
}

#[derive(Serialize)]
struct EmbeddingRequest {
    input: String,
    model: String,
}

#[derive(Deserialize, Debug)]
struct EmbeddingResponse {
    data: Option<Vec<EmbeddingData>>,
}

#[derive(Deserialize, Debug)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Serialize)]
#[allow(dead_code)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
}

#[derive(Serialize, Deserialize)]
#[allow(dead_code)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct Choice {
    message: Message,
}

#[async_trait]
impl EmbeddingEngine for OpenAIEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let safe_text = if text.len() > 8000 {
            let mut end = 8000;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            &text[..end]
        } else {
            text
        };
        let response = self.client.post(&self.url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&EmbeddingRequest {
                input: safe_text.to_string(),
                model: self.embedding_model.clone(),
            })
            .send()
            .await?;

        let status = response.status();
        let body_text = response.text().await?;
        
        let body: EmbeddingResponse = serde_json::from_str(&body_text)
            .map_err(|e| anyhow::anyhow!("JSON Error: {}. Cuerpo: {}", e, body_text))?;

        if let Some(data) = body.data {
            if let Some(first) = data.first() {
                return Ok(first.embedding.clone());
            }
        }
        Err(anyhow::anyhow!("Status {}: {}", status, body_text))
    }

    fn vector_size(&self) -> u64 { self.size }

    async fn chat(&self, system_prompt: &str, user_query: &str, context: &str) -> Result<String> {
        let full_user_content = format!("CONTEXTO DEL CODIGO:\n{}\n\nPREGUNTA: {}", context, user_query);
        let response = self.client.post(&self.chat_url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&ChatRequest {
                model: self.chat_model.clone(),
                messages: vec![
                    Message { role: "system".into(), content: system_prompt.to_string() },
                    Message { role: "user".into(), content: full_user_content },
                ],
            })
            .send()
            .await?;

        let body: ChatResponse = response.json().await?;
        Ok(body.choices.first().map(|c| c.message.content.clone()).unwrap_or_else(|| "Sin respuesta".into()))
    }
}

#[allow(dead_code)]
pub struct MockEmbedder { size: u64 }
#[allow(dead_code)]
impl MockEmbedder { pub fn new(size: u64) -> Self { Self { size } } }
#[async_trait]
impl EmbeddingEngine for MockEmbedder {
    async fn embed(&self, _: &str) -> Result<Vec<f32>> { Ok(vec![0.0; self.size as usize]) }
    fn vector_size(&self) -> u64 { self.size }
    async fn chat(&self, _: &str, _: &str, _: &str) -> Result<String> { Ok("Simulado".into()) }
}
