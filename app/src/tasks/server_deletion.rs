use crate::{Data, Error};
use chrono::Utc;
use poise::serenity_prelude as serenity;
use std::time::Duration;
use async_trait::async_trait;
use crate::tasks::Task;

pub struct ServerDeletionTask;

impl ServerDeletionTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Task for ServerDeletionTask {
    async fn run(&self, _ctx: &serenity::Context, data: Data) -> Result<(), Error> {
        loop {
            let master_key = std::env::var("ARCHON_MASTER_KEY").expect("MASTER_KEY not set");
            let client = reqwest::Client::new();

            let now = Utc::now().timestamp();
            let mut servers_to_delete = Vec::new();

            {
                let mut settings = data.settings.write().await;
                let mut changes_made = false;

                for (_user_id, user_settings) in settings.user_settings.iter_mut() {
                    let initial_len = user_settings.testing_servers.len();
                    user_settings.testing_servers.retain(|server| {
                        if server.deletion_time <= now {
                            servers_to_delete.push(server.server_id.clone());
                            false
                        } else {
                            true
                        }
                    });
                    if user_settings.testing_servers.len() != initial_len {
                        changes_made = true;
                    }
                }

                if changes_made {
                    settings.save()?;
                }
            }

            for server_id in servers_to_delete {
                let _ = client
                    .post(format!(
                        "https://archon.pyro.host/modrinth/v0/servers/{}/delete",
                        server_id
                    ))
                    .header("X-MASTER-KEY", &master_key)
                    .send()
                    .await;
            }

            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    }
}