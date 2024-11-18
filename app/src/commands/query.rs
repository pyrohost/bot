use crate::metrics::MetricsClient;
use crate::{Context, Error};
use chrono::Utc;
use poise::serenity_prelude::{Color, CreateEmbed};
use poise::CreateReply;
use std::time::Instant;

/// Executes a raw Prometheus query and returns the results in a formatted Discord embed
///
/// # Arguments
/// * `ctx` - The command context
/// * `query` - The Prometheus query string to execute
///
/// # Returns
/// * `Result<(), Error>` - Success or error status of the command execution
#[poise::command(slash_command, user_cooldown = 5)]
pub async fn query(
    ctx: Context<'_>,
    #[description = "Prometheus query to execute"] query: String,
) -> Result<(), Error> {
    // Initialize metrics client and timing
    let client = MetricsClient::new();
    let start_time = Instant::now();

    // Execute query and measure execution time
    let result = client.fetch_metric(&query).await?;
    let _ = start_time.elapsed();

    // Construct the response embed
    let message = CreateReply::default().embed(
        CreateEmbed::default()
            .title("Prometheus Query")
            .field("Query", format!("`{}`", query), false)
            .field("Result", format!("```{:.2?}```", result), false)
            .color(Color::from_rgb(255, 255, 255))
            .timestamp(Utc::now()),
    );

    // Send the response
    ctx.send(message).await?;
    Ok(())
}
