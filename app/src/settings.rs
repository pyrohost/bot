use poise::serenity_prelude::{self as serenity, ChannelId, GuildId, MessageId, RoleId, UserId};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use tracing::info;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TestingServer {
    pub server_id: String,
    pub deletion_time: i64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct GuildSettings {
    pub stats_category: Option<ChannelId>,
    pub nodes_channel: Option<ChannelId>,
    pub network_channel: Option<ChannelId>,
    pub network_total_channel: Option<ChannelId>,
    pub storage_channel: Option<ChannelId>,
    pub memory_channel: Option<ChannelId>,
    pub lorax_role: Option<RoleId>,
    pub lorax_channel: Option<ChannelId>,
    pub lorax_state: LoraxState,
}

impl GuildSettings {
    pub fn get_stats_channels(&self) -> Vec<ChannelId> {
        let mut channels = Vec::new();
        if let Some(ch) = self.nodes_channel {
            channels.push(ch);
        }
        if let Some(ch) = self.network_channel {
            channels.push(ch);
        }
        if let Some(ch) = self.network_total_channel {
            channels.push(ch);
        }
        if let Some(ch) = self.storage_channel {
            channels.push(ch);
        }
        if let Some(ch) = self.memory_channel {
            channels.push(ch);
        }
        channels
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserSettings {
    pub modrinth_id: Option<String>,
    pub testing_servers: Vec<TestingServer>,
    pub max_testing_servers: u32,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            modrinth_id: None,
            testing_servers: Vec::new(),
            max_testing_servers: 1,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub enum LoraxState {
    #[default]
    Idle,
    Submissions {
        end_time: i64,
        message_id: MessageId,
        submissions: HashMap<UserId, String>,
        location: String,
        voting_duration: u64, // Add this field
        tiebreaker_duration: u64,
    },
    Voting {
        end_time: i64,
        message_id: MessageId,
        thread_id: Option<ChannelId>, // Add this
        options: Vec<String>,
        votes: HashMap<UserId, usize>,
        submissions: HashMap<UserId, String>,
        location: String,
        tiebreaker_duration: u64,
    },
    TieBreaker {
        end_time: i64,
        message_id: MessageId,
        thread_id: Option<ChannelId>, // Add this
        options: Vec<String>,
        votes: HashMap<UserId, usize>,
        location: String,
        round: u32,
        tiebreaker_duration: u64,
        submissions: HashMap<UserId, String>,
    },
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Settings {
    pub guilds: HashMap<GuildId, GuildSettings>,
    pub user_settings: HashMap<UserId, UserSettings>,
}

impl Settings {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn load(pool: &SqlitePool) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        fn from_db<T>(db_field: Option<i64>) -> Option<T>
        where
            T: From<u64>,
        {
            db_field.map(|v| T::from(v as u64))
        }

        // We need to do some funky unpacking because the types serenity
        // gives for Ids cannot be automatically converted :^)
        let guild_rows_res = sqlx::query!(
            r#"
            select id, stats_category, nodes_channel, network_channel, network_total_channel,
                    storage_channel, memory_channel, lorax_role, lorax_channel, lorax_state
            from guilds
            "#,
        )
        .fetch_all(pool)
        .await;

        let user_rows_res = sqlx::query!(
            r#"
            select id, modrinth_id, testing_servers, max_testing_servers
            from users
            "#,
        )
        .fetch_all(pool)
        .await;

        if let (Ok(guild_rows), Ok(user_rows)) = (guild_rows_res, user_rows_res) {
            info!("Found settings...");
            let mut guild_map: HashMap<GuildId, GuildSettings> = HashMap::new();
            let mut user_map: HashMap<UserId, UserSettings> = HashMap::new();

            guild_rows.into_iter().for_each(|r| {
                guild_map.insert(
                    GuildId::new(r.id as u64),
                    GuildSettings {
                        stats_category: from_db(r.stats_category),
                        nodes_channel: from_db(r.nodes_channel),
                        network_channel: from_db(r.network_channel),
                        network_total_channel: from_db(r.network_total_channel),
                        storage_channel: from_db(r.storage_channel),
                        memory_channel: from_db(r.memory_channel),
                        lorax_role: from_db(r.lorax_role),
                        lorax_channel: from_db(r.lorax_channel),
                        lorax_state: serde_json::from_str(r.lorax_state.unwrap().as_str()).unwrap(),
                    },
                );
            });

            user_rows.into_iter().for_each(|r| {
                user_map.insert(
                    UserId::new(r.id as u64),
                    UserSettings {
                        modrinth_id: r.modrinth_id,
                        testing_servers: serde_json::from_str(r.testing_servers.unwrap().as_str())
                            .unwrap(),
                        max_testing_servers: r.max_testing_servers.unwrap_or(0) as u32,
                    },
                );
            });

            //info!("Guilds: {:#?}", guild_map);
            //info!("Users: {:#?}", user_map);

            Ok(Settings {
                guilds: guild_map,
                user_settings: user_map,
            })
        } else {
            info!("Creating new settings...");
            let settings = Self::new();
            settings.save(pool).await?;
            Ok(settings)
        }
    }

    pub async fn save(
        &self,
        pool: &SqlitePool,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        for (k, v) in self.guilds.iter() {
            let id = k.get() as i64;
            let stats_category = v.stats_category.map(|v| v.get() as i64);
            let nodes_channel = v.nodes_channel.map(|v| v.get() as i64);
            let network_channel = v.network_channel.map(|v| v.get() as i64);
            let network_total_channel = v.network_total_channel.map(|v| v.get() as i64);
            let storage_channel = v.storage_channel.map(|v| v.get() as i64);
            let memory_channel = v.memory_channel.map(|v| v.get() as i64);
            let lorax_role = v.lorax_role.map(|v| v.get() as i64);
            let lorax_channel = v.lorax_channel.map(|v| v.get() as i64);
            let lorax_state_serialized = serde_json::to_string(&v.lorax_state).unwrap();

            sqlx::query!(
                r#"
                insert into guilds (
                    id, stats_category, nodes_channel, network_channel, 
                    network_total_channel, storage_channel, memory_channel,
                    lorax_role, lorax_channel, lorax_state
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                on conflict(id) do update set
                    stats_category = excluded.stats_category,
                    nodes_channel = excluded.nodes_channel,
                    network_channel = excluded.network_channel,
                    network_total_channel = excluded.network_total_channel,
                    storage_channel = excluded.storage_channel,
                    memory_channel = excluded.memory_channel,
                    lorax_role = excluded.lorax_role,
                    lorax_channel = excluded.lorax_channel,
                    lorax_state = excluded.lorax_state
                "#,
                id,
                stats_category,
                nodes_channel,
                network_channel,
                network_total_channel,
                storage_channel,
                memory_channel,
                lorax_role,
                lorax_channel,
                lorax_state_serialized,
            )
            .execute(pool)
            .await?;
        }

        for (k, v) in self.user_settings.iter() {
            let id = k.get() as i64;
            let testing_servers_serialized = serde_json::to_string(&v.testing_servers).unwrap();

            sqlx::query!(
                r#"
                insert into users (
                    id, modrinth_id, testing_servers, max_testing_servers
                ) VALUES ($1, $2, $3, $4)
                on conflict(id) do update set
                    modrinth_id = excluded.modrinth_id,
                    testing_servers = excluded.testing_servers,
                    max_testing_servers = excluded.max_testing_servers
                "#,
                id,
                v.modrinth_id,
                testing_servers_serialized,
                v.max_testing_servers,
            )
            .execute(pool)
            .await?;
        }

        Ok(())
    }

    pub fn get_guild_settings(&self, guild_id: serenity::GuildId) -> GuildSettings {
        self.guilds.get(&guild_id).cloned().unwrap_or_default()
    }

    pub fn set_guild_settings(&mut self, guild_id: serenity::GuildId, settings: GuildSettings) {
        self.guilds.insert(guild_id, settings);
    }

    pub fn get_user_settings(&self, user_id: UserId) -> UserSettings {
        self.user_settings
            .get(&user_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_user_settings(&mut self, user_id: UserId, settings: UserSettings) {
        self.user_settings.insert(user_id, settings);
    }
}
