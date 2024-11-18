use thiserror::Error;
use poise::serenity_prelude as serenity;

#[derive(Error, Debug)]
pub enum BotError {
    #[error("Discord API error: {0}")]
    Discord(#[from] serenity::Error),

    #[error("Metrics error: {0}")]
    Metrics(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
}

impl From<&str> for BotError {
    fn from(s: &str) -> Self {
        BotError::Metrics(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, BotError>;