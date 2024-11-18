use poise::serenity_prelude as serenity;
use std::{collections::HashMap, time::Duration};
use tokio::time::{self, MissedTickBehavior};
use tracing::{error, info};
use async_trait::async_trait;

use crate::{
    metrics::{Metric, MetricsClient, METRICS},
    Data, Error,
};

use super::TaskHandler;

const UPDATE_INTERVAL: Duration = Duration::from_secs(60 * 5);
const RATE_LIMIT_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug)]
pub struct ChannelUpdater {
    previous_names: HashMap<serenity::ChannelId, String>,
    metrics_client: MetricsClient,
}

impl ChannelUpdater {
    fn new() -> Self {
        Self {
            previous_names: HashMap::new(),
            metrics_client: MetricsClient::new(),
        }
    }

    async fn update_if_changed(
        &mut self,
        ctx: &serenity::Context,
        channel_id: serenity::ChannelId,
        new_name: String,
    ) -> std::result::Result<(), Error> {
        if self
            .previous_names
            .get(&channel_id)
            .map_or(true, |prev| prev != &new_name)
        {
            channel_id
                .edit(&ctx.http, serenity::EditChannel::default().name(&new_name))
                .await?;
            self.previous_names.insert(channel_id, new_name);
        }
        Ok(())
    }

    async fn update_metric(
        &mut self,
        ctx: &serenity::Context,
        channel_id: Option<serenity::ChannelId>,
        metric: &Metric,
    ) -> std::result::Result<(), Error> {
        if let Some(channel) = channel_id {
            let value = self.metrics_client.fetch_metric(metric.query).await?;
            let name = metric.format_value(value);
            self.update_if_changed(ctx, channel, name).await?;
            time::sleep(Duration::from_millis(500)).await;
        }
        Ok(())
    }

    async fn update_guild_metrics(
        &mut self,
        ctx: &serenity::Context,
        guild_id: serenity::GuildId,
        channels: &[Option<serenity::ChannelId>],
    ) -> std::result::Result<(), Error> {
        info!("Updating stats for guild {}", guild_id);

        for (metric, &channel) in METRICS.iter().zip(channels.iter()) {
            time::sleep(RATE_LIMIT_DELAY).await;
            if let Err(e) = self.update_metric(ctx, channel, metric).await {
                error!(
                    "Failed to update {} metric for guild {}: {}",
                    metric.name, guild_id, e
                );
                return Err(e);
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct StatsUpdaterTask {
    updater: ChannelUpdater,
}

impl StatsUpdaterTask {
    pub fn new() -> Self {
        Self {
            updater: ChannelUpdater::new(),
        }
    }
}

#[async_trait]
impl TaskHandler for StatsUpdaterTask {
    fn name(&self) -> &'static str {
        "stats_updater"
    }

    async fn run(&mut self, ctx: &serenity::Context, data: Data) -> std::result::Result<(), Error> {
        let mut interval = time::interval(UPDATE_INTERVAL);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            info!("Starting channel update cycle");

            let guild_settings = data.settings.read().await.guilds.clone();

            for (guild_id, settings) in guild_settings {
                let channels = [
                    settings.nodes_channel,
                    settings.network_channel,
                    settings.network_total_channel,
                    settings.storage_channel,
                    settings.memory_channel,
                ];

                if let Err(e) = self.updater.update_guild_metrics(ctx, guild_id, &channels).await {
                    tracing::error!("Failed to update metrics: {}", e);
                }
            }
        }
    }
}
