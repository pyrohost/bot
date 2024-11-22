pub mod stats_updater;
pub mod lorax_scheduler;

use async_trait::async_trait;
use poise::serenity_prelude as serenity;
use crate::{Data, Error};

#[async_trait]
pub trait TaskHandler: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    async fn run(&mut self, ctx: &serenity::Context, data: Data) -> Result<(), Error>;
}

pub struct TaskManager {
    tasks: Vec<Box<dyn TaskHandler>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    pub fn register_task<T: TaskHandler>(&mut self, task: T) {
        self.tasks.push(Box::new(task));
    }

    pub async fn run_all(self, ctx: &serenity::Context, data: Data) {
        for mut task in self.tasks {
            let task_name = task.name().to_string();
            let ctx = ctx.clone();
            let data = data.clone();

            tokio::spawn(async move {
                if let Err(e) = task.run(&ctx, data).await {
                    tracing::error!("Task {} failed: {}", task_name, e);
                }
            });
        }
    }
}