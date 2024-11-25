use crate::{Data, Error};
use async_trait::async_trait;
use poise::serenity_prelude as serenity;
use std::sync::Arc;

pub mod lorax_scheduler;
pub mod server_deletion;
pub mod stats_updater;

#[async_trait]
pub trait Task: Send + Sync + 'static {
    async fn run(
        &self,
        ctx: &serenity::Context,
        data: Data,
    ) -> Result<(), Error>;
}

pub struct TaskManager {
    tasks: Vec<Arc<dyn Task>>,
}

impl TaskManager {
    pub fn new() -> Self {
        TaskManager { tasks: Vec::new() }
    }

    pub fn register_task<T: Task>(&mut self, task: T) {
        self.tasks.push(Arc::new(task));
    }

    pub async fn run_all(&self, ctx: &serenity::Context, data: Data) {
        for task in &self.tasks {
            let task = Arc::clone(task);
            let ctx = ctx.clone();
            let data = data.clone();
            tokio::spawn(async move {
                if let Err(e) = task.run(&ctx, data).await {
                    eprintln!("Error in task: {}", e);
                }
            });
        }
    }
}
