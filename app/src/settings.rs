use poise::serenity_prelude::{self as serenity, UserId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct GuildSettings {
    pub stats_category: Option<serenity::ChannelId>,
    pub nodes_channel: Option<serenity::ChannelId>,
    pub network_channel: Option<serenity::ChannelId>,
    pub network_total_channel: Option<serenity::ChannelId>,
    pub storage_channel: Option<serenity::ChannelId>,
    pub memory_channel: Option<serenity::ChannelId>,
    pub lorax_role: Option<serenity::RoleId>,
    pub lorax_channel: Option<serenity::ChannelId>,
    pub lorax_state: LoraxState,
}

impl GuildSettings {
    pub fn get_stats_channels(&self) -> Vec<serenity::ChannelId> {
        let mut channels = Vec::new();
        if let Some(ch) = self.nodes_channel { channels.push(ch); }
        if let Some(ch) = self.network_channel { channels.push(ch); }
        if let Some(ch) = self.network_total_channel { channels.push(ch); }
        if let Some(ch) = self.storage_channel { channels.push(ch); }
        if let Some(ch) = self.memory_channel { channels.push(ch); }
        channels
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct UserSettings {
    pub modrinth_id: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LoraxState {
    Idle,
    Submissions {
        end_time: i64,
        message_id: serenity::MessageId,
        submissions: HashMap<UserId, String>,
    },
    Voting {
        end_time: i64,
        message_id: serenity::MessageId,
        options: Vec<String>,
        votes: HashMap<UserId, usize>,
    },
}

impl Default for LoraxState {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Settings {
    pub guilds: HashMap<serenity::GuildId, GuildSettings>,
    #[serde(skip)]
    file_path: PathBuf,
    user_settings: HashMap<UserId, UserSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            guilds: HashMap::new(),
            file_path: PathBuf::from("settings.json"),
            user_settings: HashMap::new(),
        }
    }
}

impl Settings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let file_path = PathBuf::from("settings.json");
        let settings = if file_path.exists() {
            let data = fs::read_to_string(&file_path)?;
            let mut settings: Settings = serde_json::from_str(&data)?;
            settings.file_path = file_path;
            settings
        } else {
            let settings = Self::new();
            settings.save()?;
            settings
        };
        Ok(settings)
    }

    pub fn save(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let data = serde_json::to_string_pretty(self)?;
        fs::write(&self.file_path, &data)?;
        Ok(())
    }

    pub fn get_guild_settings(&self, guild_id: serenity::GuildId) -> GuildSettings {
        self.guilds.get(&guild_id).cloned().unwrap_or_default()
    }

    pub fn set_guild_settings(&mut self, guild_id: serenity::GuildId, settings: GuildSettings) {
        self.guilds.insert(guild_id, settings);
    }

    pub fn get_user_settings(&self, user_id: UserId) -> UserSettings {
        self.user_settings.get(&user_id).cloned().unwrap_or_default()
    }

    pub fn set_user_settings(&mut self, user_id: UserId, settings: UserSettings) {
        self.user_settings.insert(user_id, settings);
    }
}