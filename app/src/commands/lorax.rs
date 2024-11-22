use crate::metrics::MetricsClient;
use crate::settings::LoraxState;
use crate::{Context, Data, Error};
use chrono::{Duration, Utc};
use poise::serenity_prelude::{
    self as serenity, futures, ButtonStyle, ChannelId, Color, ComponentInteraction,
    CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter,
    CreateInteractionResponseMessage, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption, RoleId,
};
use poise::CreateReply;
use rand::Rng;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

#[poise::command(
    slash_command,
    subcommands(
        "set_role",
        "set_channel",
        "start",
        "submit",
        "vote",
        "list",
        "cancel",
        "duration",
        "status",
        "remove",
        "force_end",
    )
)]
pub async fn lorax(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_CHANNELS")]
pub async fn set_channel(
    ctx: Context<'_>,
    #[description = "Channel for node naming announcements"] channel: ChannelId,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    {
        let mut settings = ctx.data().settings.write().await;
        let mut guild_settings = settings.get_guild_settings(guild_id);
        guild_settings.lorax_channel = Some(channel);
        settings.set_guild_settings(guild_id, guild_settings);
        settings.save()?;
    }

    ctx.say(format!(
        "Got it! I'll post all future Lorax announcements in <#{}>. 🌳",
        channel
    ))
    .await?;

    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_ROLES")]
pub async fn set_role(
    ctx: Context<'_>,
    #[description = "Role to ping for node naming events"] role: RoleId,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    {
        let mut settings = ctx.data().settings.write().await;
        let mut guild_settings = settings.get_guild_settings(guild_id);
        guild_settings.lorax_role = Some(role);
        settings.set_guild_settings(guild_id, guild_settings);
        settings.save()?;
    }

    ctx.say(format!(
        "Great! Members with the <@&{}> role will now get notifications about Lorax events. 🌿",
        role
    ))
    .await?;

    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn start(
    ctx: Context<'_>,
    #[description = "Location of the new node (e.g., 'US-East', 'EU-West')"] location: String,
    #[description = "Submission duration in minutes"] submission_duration: Option<u64>,
    #[description = "Voting duration in minutes"] voting_duration: Option<u64>,
    #[description = "Tiebreaker duration in minutes"] tiebreaker_duration: Option<u64>,
) -> Result<(), Error> {
    let submission_duration = submission_duration.unwrap_or(60);
    let voting_duration = voting_duration.unwrap_or(60);
    let tiebreaker_duration = tiebreaker_duration.unwrap_or(30);
    let guild_id = ctx.guild_id().unwrap();
    let data = ctx.data().clone();

    let mut settings = data.settings.write().await;
    let guild_settings = settings.get_guild_settings(guild_id);

    if guild_settings.lorax_channel.is_none() || guild_settings.lorax_role.is_none() {
        ctx.say("Hold on! Please set the Lorax channel and role before starting an event. Use `/lorax set_channel` and `/lorax set_role`.").await?;
        return Ok(());
    }

    if let LoraxState::Idle = guild_settings.lorax_state {
        let end_time = Utc::now().timestamp() + (submission_duration * 60) as i64;

        let role_id = guild_settings.lorax_role.unwrap();
        let announcement = format!(
            "Hey <@&{}>! We're launching a new node in **{}**, and we need your help to name it! 🌳\n\n\
            Got any cool tree names in mind? Submit your idea using `/lorax submit`! Just make sure it's all lowercase letters, like `oak` or `willow`.\n\n\
            You've got until {} to send in your suggestions. Good luck!",
            role_id,
            location,
            discord_timestamp(end_time, TimestampStyle::ShortDateTime)
        );

        let channel_id = guild_settings.lorax_channel.unwrap();
        let announcement_msg = channel_id.say(&ctx, announcement).await?;

        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Submissions {
            end_time,
            message_id: announcement_msg.id,
            submissions: HashMap::new(),
            location,
            tiebreaker_duration,
        };
        settings.save()?;

        let data = ctx.data().clone();
        let ctx_clone = ctx.serenity_context().clone();
        tokio::spawn(async move {
            let seconds = submission_duration as i64 * 60;
            tokio::time::sleep(Duration::seconds(seconds).to_std().unwrap()).await;

            if let Err(e) = start_voting(&ctx_clone, &data, guild_id, voting_duration).await {
                tracing::error!("Failed to start voting: {}", e);
            }
        });

        ctx.say("🎉 Lorax event started! Submissions are now open.")
            .await?;
    } else {
        ctx.say("⚠️ A Lorax event is already in progress.").await?;
    }

    Ok(())
}

pub async fn start_voting(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: serenity::GuildId,
    voting_duration: u64,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    let state = &settings.guilds.get(&guild_id).unwrap().lorax_state;

    let channel_id = settings
        .guilds
        .get(&guild_id)
        .unwrap()
        .lorax_channel
        .unwrap();
    let role_id = settings.guilds.get(&guild_id).unwrap().lorax_role.unwrap();

    if let LoraxState::Submissions {
        submissions,
        location,
        tiebreaker_duration,
        ..
    } = state
    {
        if submissions.is_empty() {
            channel_id
                .say(
                    ctx,
                    "🌳 Hmm, looks like we didn't get any submissions this time. :(",
                )
                .await?;
            settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Idle;
            settings.save()?;
            return Ok(());
        }

        let end_time = Utc::now().timestamp() + (voting_duration * 60) as i64;
        let options = submissions.values().cloned().collect::<Vec<_>>();
        let submission_count = options.len();

        let announcement = format!(
            "Hey <@&{}>! It's voting time for our new **{}** node! 🗳️\n\n\
            We've got {} awesome name suggestions. Use `/lorax vote` to pick your favorite!\n\n\
            Voting ends {}.",
            role_id,
            location,
            submission_count,
            discord_timestamp(end_time, TimestampStyle::ShortDateTime)
        );

        let announcement_msg = channel_id.say(ctx, announcement).await?;

        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Voting {
            end_time,
            message_id: announcement_msg.id,
            options,
            votes: HashMap::new(),

            submissions: submissions.clone(),
            location: location.clone(),
            tiebreaker_duration: *tiebreaker_duration,
        };
        settings.save()?;

        let data_clone = data.clone();
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            let seconds = voting_duration as i64 * 60;
            tokio::time::sleep(Duration::seconds(seconds).to_std().unwrap()).await;
            if let Err(e) = announce_winner(&ctx_clone.http, &data_clone, guild_id).await {
                tracing::error!("Failed to announce winner: {}", e);
            }
        });
    }

    Ok(())
}

pub async fn start_tiebreaker(
    http: &Arc<serenity::Http>,
    data: &Data,
    guild_id: serenity::GuildId,
    tied_options: Vec<(usize, String)>,
    location: String,
    round: u32,
    tiebreaker_duration: u64,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    let guild = settings.guilds.get_mut(&guild_id).unwrap();

    let channel_id = guild.lorax_channel.unwrap();
    let role_id = guild.lorax_role.unwrap();

    let end_time = Utc::now().timestamp() + (tiebreaker_duration * 60) as i64;
    let options: Vec<String> = tied_options.into_iter().map(|(_, name)| name).collect();

    let submissions = match &guild.lorax_state {
        LoraxState::Voting { submissions, .. } => submissions.clone(),
        _ => HashMap::new(),
    };

    let announcement = format!(
        "🎯 Hey <@&{}>! We've got a tie! Time for tiebreaker round {}!\n\n\
        The following names are tied:\n{}\n\n\
        Use `/lorax vote` to break the tie! One name will be eliminated.\n\n\
        This round ends {}.",
        role_id,
        round,
        options
            .iter()
            .map(|name| format!("• `{}`", name))
            .collect::<Vec<_>>()
            .join("\n"),
        discord_timestamp(end_time, TimestampStyle::ShortDateTime)
    );

    let announcement_msg = channel_id.say(http, announcement).await?;

    guild.lorax_state = LoraxState::TieBreaker {
        end_time,
        message_id: announcement_msg.id,
        options: options.clone(),
        votes: HashMap::new(),
        location: location.clone(),
        round,
        tiebreaker_duration,
        submissions,
    };
    settings.save()?;
    drop(settings);

    let http = http.clone();

    let data = data.clone();
    
    // this might not even be neccesary If I fucking do tasks right
    // ideally task* managers should check application state every x time (likely 1 minute)
    // and be like, oh there's a running event! how long is it ? and handle it that way
    // this is just dirty
    // - ellie

    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(tiebreaker_duration * 60)).await;
        if let Err(e) = announce_winner(&http, &data, guild_id).await {
            error!("Failed to announce tiebreaker results: {}", e);
        }
    });

    Ok(())
}

async fn eliminate_random_tree(options: &[String]) -> usize {
    let mut rng = rand::thread_rng();
    rng.gen_range(0..options.len())
}

pub async fn announce_winner(
    http: &Arc<serenity::Http>,
    data: &Data,
    guild_id: serenity::GuildId,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    let guild = settings.guilds.get_mut(&guild_id).unwrap();
    let channel_id = guild.lorax_channel.unwrap();
    let state = guild.lorax_state.clone();

    match state {
        LoraxState::Voting {
            options,
            votes,
            submissions,
            location,
            tiebreaker_duration,
            ..
        }
        | LoraxState::TieBreaker {
            options,
            votes,
            location,
            round: _,
            tiebreaker_duration,
            ..
        } => {
            let mut vote_counts: HashMap<usize, usize> = HashMap::new();
            for &choice in votes.values() {
                *vote_counts.entry(choice).or_insert(0) += 1;
            }

            if vote_counts.is_empty() || options.len() <= 1 {
                if let Some(winning_tree) = options.get(0) {
                    channel_id
                        .say(
                            http,
                            format!(
                                "🎉 The winning tree name is **{}**! This will be the name for our new **{}** node. Thank you all for participating!",
                                winning_tree, location
                            ),
                        )
                        .await?;

                    guild.lorax_state = LoraxState::Idle;
                    settings.save()?;
                } else {
                    channel_id
                        .say(
                            http,
                            "No valid tree names remain. The event has ended without a winner.",
                        )
                        .await?;

                    guild.lorax_state = LoraxState::Idle;
                    settings.save()?;
                }
                return Ok(());
            }

            let max_votes = vote_counts.values().max().unwrap_or(&0);
            let tied_options: Vec<(usize, String)> = options
                .iter()
                .enumerate()
                .filter(|(i, _)| vote_counts.get(i).unwrap_or(&0) == max_votes)
                .map(|(i, name)| (i, name.clone()))
                .collect();

            if tied_options.len() > 1 {
                let min_votes = vote_counts.values().min().unwrap_or(&0);
                let lowest_options: Vec<usize> = options
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| vote_counts.get(i).unwrap_or(&0) == min_votes)
                    .map(|(i, _)| i)
                    .collect();

                let remove_idx = {
                    let mut rng = rand::thread_rng();
                    lowest_options[rng.gen_range(0..lowest_options.len())]
                };

                let remaining_options: Vec<(usize, String)> = options
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != remove_idx)
                    .map(|(i, name)| (i, name.clone()))
                    .collect();

                let round = if let LoraxState::TieBreaker { round, .. } = guild.lorax_state {
                    round + 1
                } else {
                    1
                };

                drop(settings);

                return start_tiebreaker(
                    http,
                    data,
                    guild_id,
                    remaining_options,
                    location.clone(),
                    round,
                    tiebreaker_duration,
                )
                .await;
            }

            if let Some((_, winning_tree)) = tied_options.first() {
                channel_id
                    .say(
                        http,
                        format!(
                            "🎉 The winning tree name is **{}**! This will be the name for our new **{}** node. Thank you all for participating!",
                            winning_tree, location
                        ),
                    )
                    .await?;

                guild.lorax_state = LoraxState::Idle;
                settings.save()?;
            }
        }
        _ => {}
    }

    Ok(())
}

fn discord_timestamp(time: i64, style: TimestampStyle) -> String {
    format!("<t:{}:{}>", time, style.as_str())
}

enum TimestampStyle {
    ShortTime,
    LongTime,
    ShortDate,
    LongDate,
    ShortDateTime,
    LongDateTime,
    Relative,
}

impl TimestampStyle {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ShortTime => "t",
            Self::LongTime => "T",
            Self::ShortDate => "d",
            Self::LongDate => "D",
            Self::ShortDateTime => "f",
            Self::LongDateTime => "F",
            Self::Relative => "R",
        }
    }
}

const RESERVED_NAMES: &[&str] = &[
    "sakura", "cherry", "bamboo", "maple", "pine", "palm", "cedar",
];

fn validate_tree_name(name: &str) -> Result<(), &'static str> {
    if name.len() < 3 || name.len() > 20 {
        return Err("Tree name must be between 3 and 20 characters long.");
    }
    if !name.chars().all(|c| c.is_ascii_lowercase()) {
        return Err("Only lowercase ASCII letters are allowed, with no spaces.");
    }
    if RESERVED_NAMES.contains(&name) {
        return Err("This tree name is reserved for future use.");
    }
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn cancel(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;

    match &settings.guilds.get(&guild_id).unwrap().lorax_state {
        LoraxState::Idle => {
            ctx.say("No active Lorax event to cancel.").await?;
            return Ok(());
        }
        _ => {
            let channel_id = settings
                .guilds
                .get(&guild_id)
                .unwrap()
                .lorax_channel
                .unwrap();
            channel_id
                .say(
                    &ctx,
                    "🚫 The current Lorax event has been cancelled by an administrator.",
                )
                .await?;
            settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Idle;
            settings.save()?;
            ctx.say("Alright, the current Lorax event has been cancelled and reset. 🛑")
                .await?;
        }
    }
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn duration(
    ctx: Context<'_>,
    #[description = "Minutes to adjust (positive to extend, negative to reduce)"] minutes: i64,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;

    match &mut settings.guilds.get_mut(&guild_id).unwrap().lorax_state {
        LoraxState::Idle => {
            ctx.say("No active Lorax event to modify.").await?;
        }
        LoraxState::Submissions { end_time, .. }
        | LoraxState::Voting { end_time, .. }
        | LoraxState::TieBreaker { end_time, .. } => {
            let current_time = Utc::now().timestamp();
            let new_end = *end_time + (minutes * 60);

            if new_end <= current_time {
                ctx.say(
                    "Hmm, I can't set the end time to the past. Let's try a different amount. ⏳",
                )
                .await?;
                return Ok(());
            }

            *end_time = new_end;

            let channel = settings.guilds.get(&guild_id).unwrap().lorax_channel;
            drop(settings);

            if let Some(channel_id) = channel {
                let message = if minutes > 0 {
                    format!(
                        "⏰ The current phase has been extended by {} minutes! More time to get involved.",
                        minutes
                    )
                } else {
                    format!(
                        "⏰ The current phase has been shortened by {} minutes. Don't miss out!",
                        minutes.abs()
                    )
                };

                channel_id
                    .say(
                        &ctx,
                        format!(
                            "{} New end time: {}",
                            message,
                            discord_timestamp(new_end, TimestampStyle::ShortDateTime)
                        ),
                    )
                    .await?;

                ctx.say("Got it! The duration has been updated. 📅").await?;
            }
        }
    }
    Ok(())
}

#[poise::command(slash_command, ephemeral)]
pub async fn submit(
    ctx: Context<'_>,
    #[description = "Your tree name suggestion (lowercase letters only)"] name: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let user_id = ctx.author().id;
    let tree_name = name.to_lowercase();

    if let Err(msg) = validate_tree_name(&tree_name) {
        ctx.say(format!("Oops! {}", msg)).await?;
        return Ok(());
    }

    let metrics_client = MetricsClient::new();
    let existing_trees = metrics_client.fetch_existing_trees().await?;
    if existing_trees.contains(&tree_name) {
        ctx.say("This tree name is already in use by an existing node.")
            .await?;
        return Ok(());
    }

    let mut settings = ctx.data().settings.write().await;

    if let LoraxState::Submissions { submissions, .. } =
        &mut settings.guilds.get_mut(&guild_id).unwrap().lorax_state
    {
        if submissions.values().any(|n| n == &tree_name) {
            ctx.say("This tree name has already been submitted by someone else.")
                .await?;
            return Ok(());
        }

        let msg = if submissions.contains_key(&user_id) {
            submissions.insert(user_id, tree_name);
            "Awesome! I've updated your submission. Good luck! 🌲"
        } else {
            submissions.insert(user_id, tree_name);
            "Thanks for your submission! Good luck! 🌴"
        };

        settings.save()?;
        ctx.say(msg).await?;
    } else {
        ctx.say("Submissions are not currently open.").await?;
    }

    Ok(())
}

#[poise::command(slash_command, ephemeral)]
pub async fn vote(
    ctx: Context<'_>,
    #[description = "Page number"] page: Option<u32>,
    #[description = "Search for specific names"] search: Option<String>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    info!("Processing vote command for guild {}", guild_id);
    let user_id = ctx.author().id;
    let settings = ctx.data().settings.read().await;
    let page = page.unwrap_or(1).max(1);
    const ITEMS_PER_PAGE: usize = 10;

    if let Some(guild) = settings.guilds.get(&guild_id) {
        if let LoraxState::Voting {
            options,
            votes,
            end_time,
            submissions,
            ..
        } = &guild.lorax_state
        {
            if Utc::now().timestamp() > *end_time {
                ctx.say("Voting period has ended.").await?;
                return Ok(());
            }

            let filtered_options: Vec<_> = if let Some(ref search) = search {
                options
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| name.contains(&search.to_lowercase()))
                    .collect()
            } else {
                options.iter().enumerate().collect()
            };

            let total_pages = (filtered_options.len() + ITEMS_PER_PAGE - 1) / ITEMS_PER_PAGE;
            let start_idx = (page as usize - 1) * ITEMS_PER_PAGE;
            let end_idx = start_idx + ITEMS_PER_PAGE;
            let page_options = &filtered_options
                [start_idx.min(filtered_options.len())..end_idx.min(filtered_options.len())];

            let mut components = Vec::new();

            let select_options = futures::future::join_all(page_options.iter().map(|(i, name)| {
                let ctx = ctx.clone();
                let submissions = submissions.clone();
                async move {
                    let submitter_name = get_submitter_name(
                        ctx.serenity_context(),
                        *submissions.iter().find(|(_, n)| n == name).unwrap().0,
                    )
                    .await;
                    CreateSelectMenuOption::new(
                        format!("{} - Submitted by {}", name, submitter_name),
                        i.to_string(),
                    )
                    .default_selection(votes.get(&user_id) == Some(&i))
                }
            }))
            .await;

            if !select_options.is_empty() {
                let select_menu = CreateSelectMenu::new(
                    "vote_select",
                    CreateSelectMenuKind::String {
                        options: select_options,
                    },
                )
                .custom_id(format!("vote_select_{}", page))
                .placeholder("Select a tree name to vote for")
                .max_values(1);

                components.push(CreateActionRow::SelectMenu(select_menu));
            }

            if total_pages > 1 {
                let mut nav_buttons = Vec::new();
                if page > 1 {
                    nav_buttons.push(
                        CreateButton::new(format!("vote_page_{}", page - 1))
                            .label("Previous")
                            .style(ButtonStyle::Secondary),
                    );
                }
                if page < total_pages as u32 {
                    nav_buttons.push(
                        CreateButton::new(format!("vote_page_{}", page + 1))
                            .label("Next")
                            .style(ButtonStyle::Secondary),
                    );
                }
                if !nav_buttons.is_empty() {
                    components.push(CreateActionRow::Buttons(nav_buttons));
                }
            }

            let header = format!(
                "🗳️ **Voting is now open!**\n\nSelect your favorite tree name from the menu below.\n\n_Voting ends {}_\n",
                discord_timestamp(*end_time, TimestampStyle::Relative)
            );

            ctx.send(
                CreateReply::default()
                    .content(header)
                    .components(components)
                    .ephemeral(true),
            )
            .await?;
        } else {
            ctx.say("⚠️ Voting is not currently open. Stay tuned!")
                .await?;
        }
    }

    Ok(())
}

async fn get_submitter_name(ctx: &serenity::Context, user_id: serenity::UserId) -> String {
    match user_id.to_user(ctx).await {
        Ok(user) => user.name,
        Err(_) => {
            warn!("Failed to fetch user name for user ID {}", user_id);
            "Unknown User".to_string()
        }
    }
}

pub async fn handle_button(
    ctx: &serenity::Context,
    component: &ComponentInteraction,
    data: Data,
) -> Result<(), Error> {
    debug!("Handling button interaction: {}", component.data.custom_id);

    if component.data.custom_id.starts_with("vote_page_") {
        if let Ok(_page) = component.data.custom_id[10..].parse::<u32>() {
            let builder = serenity::CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::default()
                    .content("Loading...".to_string())
                    .ephemeral(true),
            );
            component.create_response(ctx, builder).await?;
        }
        return Ok(());
    }

    if component.data.custom_id.starts_with("vote_select_") {
        let choice = match &component.data.kind {
            serenity::ComponentInteractionDataKind::StringSelect { values } => {
                values.first().and_then(|v| v.parse::<usize>().ok())
            }
            _ => None,
        };

        if let Some(choice) = choice {
            let guild_id = component.guild_id.unwrap();
            let user_id = component.user.id;

            let mut settings = data.settings.write().await;
            let response = if let Some(guild) = settings.guilds.get_mut(&guild_id) {
                if let LoraxState::Voting {
                    votes,
                    options,
                    end_time,
                    submissions,
                    ..
                } = &mut guild.lorax_state
                {
                    if Utc::now().timestamp() > *end_time {
                        info!("Rejecting vote: voting period ended");
                        return Ok(());
                    }

                    if let Some(selected_tree) = options.get(choice).cloned() {
                        if let Some((submitter_id, _)) =
                            submissions.iter().find(|(_, tree)| *tree == &selected_tree)
                        {
                            if *submitter_id == user_id {
                                debug!("User {} attempted to vote for own submission", user_id);
                                "You can't vote for your own submission!".to_string()
                            } else {
                                let previous_vote = votes.insert(user_id, choice);
                                settings.save()?;

                                if previous_vote.is_some() {
                                    format!(
                                        "👍 Got it! You've changed your vote to `{}`.",
                                        selected_tree
                                    )
                                } else {
                                    format!("👍 Thanks! You've voted for `{}`.", selected_tree)
                                }
                            }
                        } else {
                            "Invalid selection".to_string()
                        }
                    } else {
                        "Invalid selection".to_string()
                    }
                } else {
                    "Voting is not currently active".to_string()
                }
            } else {
                "Server not found".to_string()
            };

            let builder = serenity::CreateInteractionResponse::Message(
                CreateInteractionResponseMessage::default()
                    .content(response)
                    .ephemeral(true),
            );
            component.create_response(ctx, builder).await?;
        }
    }

    Ok(())
}

#[poise::command(slash_command)]
pub async fn status(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;
    let guild_settings = settings.get_guild_settings(guild_id);

    let status_msg = match &guild_settings.lorax_state {
        LoraxState::Idle => {
            "🌳 No naming event running right now. Start one with `/lorax start` and let's find a name for our next node!".to_string()
        }
        LoraxState::Submissions {
            end_time,
            submissions,
            location,
            ..
        } => {
            format!(
                "🌱 We're naming our new **{}** node!\n\nGot a great idea? Submit it with `/lorax submit` before {}!",
                location,
                discord_timestamp(*end_time, TimestampStyle::Relative),
            )
        }
        LoraxState::Voting {
            end_time,
            options,
            votes,
            location,
            ..
        } => {
            format!(
                "🗳️ Voting is underway for our **{}** node's name!\n\nWe've got {} great options, and {} votes so far.\n\nUse `/lorax vote` to have your say before {}!",
                location,
                options.len(),
                votes.len(),
                discord_timestamp(*end_time, TimestampStyle::Relative),
            )
        }
        LoraxState::TieBreaker { end_time, options, votes, location, round, .. } => {
            format!(
                "🎯 Tiebreaker Round {} is underway for our **{}** node!\n\n{} options remain, with {} votes cast.\n\nUse `/lorax vote` to break the tie before {}!",
                round,
                location,
                options.len(),
                votes.len(),
                discord_timestamp(*end_time, TimestampStyle::Relative),
            )
        }
    };

    ctx.say(status_msg).await?;
    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD", ephemeral)]
pub async fn list(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;

    let guild = ctx.guild().unwrap().clone();
    let is_admin = guild
        .member(ctx.http(), ctx.author().id)
        .await?
        .permissions(ctx)?
        .manage_guild();

    if let Some(guild) = settings.guilds.get(&guild_id) {
        match &guild.lorax_state {
            LoraxState::Submissions {
                submissions,
                end_time,
                ..
            } => {
                if (!is_admin) {
                    ctx.say(
                        "Only administrators can view submissions during the submission phase.",
                    )
                    .await?;
                    return Ok(());
                }

                let submission_list =
                    futures::future::join_all(submissions.iter().map(|(user_id, name)| {
                        let ctx = ctx.clone();
                        async move {
                            let submitter_name =
                                get_submitter_name(ctx.serenity_context(), *user_id).await;
                            format!("• `{}` (by {})", name, submitter_name)
                        }
                    }))
                    .await
                    .join("\n");

                ctx.send(
                    CreateReply::default()
                        .embed(
                            CreateEmbed::default()
                                .title("🌱 Current Submissions")
                                .description(if submission_list.is_empty() {
                                    "No submissions yet.".to_string()
                                } else {
                                    submission_list
                                })
                                .footer(CreateEmbedFooter::new(format!(
                                    "Submissions close {}",
                                    discord_timestamp(*end_time, TimestampStyle::Relative)
                                )))
                                .color(Color::from_rgb(67, 160, 71)),
                        )
                        .ephemeral(true),
                )
                .await?;
            }
            LoraxState::Voting {
                options,
                votes,
                end_time,
                submissions,
                ..
            }
            | LoraxState::TieBreaker {
                options,
                votes,
                end_time,
                submissions: _,
                ..
            } => {
                if (!is_admin) {
                    let options_list = options
                        .iter()
                        .map(|name| format!("• {}", name))
                        .collect::<Vec<_>>()
                        .join("\n");

                    ctx.send(
                        CreateReply::default().embed(
                            CreateEmbed::default()
                                .title("🌳 Submitted Tree Names")
                                .description(options_list)
                                .footer(CreateEmbedFooter::new(format!(
                                    "Voting closes {} • Total: {}",
                                    discord_timestamp(*end_time, TimestampStyle::ShortDateTime),
                                    options.len()
                                )))
                                .color(Color::from_rgb(67, 160, 71)),
                        ),
                    )
                    .await?;
                } else {
                    let mut vote_counts: HashMap<usize, usize> = HashMap::new();
                    for &choice in votes.values() {
                        *vote_counts.entry(choice).or_insert(0) += 1;
                    }

                    let mut options_with_votes: Vec<_> = options
                        .iter()
                        .enumerate()
                        .map(|(idx, name)| {
                            let votes = vote_counts.get(&idx).unwrap_or(&0);
                            (name, *votes)
                        })
                        .collect();

                    options_with_votes.sort_by(|a, b| b.1.cmp(&a.1));

                    let options_list = options_with_votes
                        .iter()
                        .map(|(name, votes)| format!("• {} ({} votes)", name, votes))
                        .collect::<Vec<_>>()
                        .join("\n");

                    ctx.send(
                        CreateReply::default().embed(
                            CreateEmbed::default()
                                .title("🌳 Current Voting Status")
                                .description(if options_list.is_empty() {
                                    "No submissions yet.".to_string()
                                } else {
                                    options_list
                                })
                                .footer(CreateEmbedFooter::new(format!(
                                    "Voting closes {} • Total votes: {}",
                                    discord_timestamp(*end_time, TimestampStyle::ShortDateTime),
                                    votes.len()
                                )))
                                .color(Color::from_rgb(67, 160, 71)),
                        ),
                    )
                    .await?;
                }
            }
            LoraxState::Idle => {
                ctx.say("No active tree naming event.").await?;
            }
        }
    }

    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_MESSAGES")]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "Tree name to remove"] tree_name: String,
    #[description = "Reason for removal"] reason: Option<String>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;
    let guild = settings.guilds.get_mut(&guild_id).unwrap();

    match &mut guild.lorax_state {
        LoraxState::Submissions { submissions, .. } => {
            if let Some((user_id, _)) = submissions.iter().find(|(_, name)| name == &&tree_name) {
                let user_id = *user_id;
                submissions.remove(&user_id);
                settings.save()?;

                let msg = if let Some(reason) = reason {
                    format!(
                        "✅ Removed submission `{}` (Reason: {}).",
                        tree_name, reason
                    )
                } else {
                    format!("✅ Removed submission `{}`.", tree_name)
                };
                ctx.say(msg).await?;
            } else {
                ctx.say("❌ Submission not found.").await?;
            }
        }
        LoraxState::Voting {
            options,
            votes,
            submissions,
            ..
        } => {
            if let Some(index) = options.iter().position(|name| name == &tree_name) {
                options.remove(index);

                votes.retain(|_, &mut vote_idx| vote_idx != index);

                for vote_idx in votes.values_mut() {
                    if *vote_idx > index {
                        *vote_idx -= 1;
                    }
                }

                submissions.retain(|_, name| name != &tree_name);
                settings.save()?;

                let msg = if let Some(reason) = reason {
                    format!(
                        "✅ Removed submission `{}` and updated votes (Reason: {})",
                        tree_name, reason
                    )
                } else {
                    format!("✅ Removed submission `{}` and updated votes", tree_name)
                };
                ctx.say(msg).await?;
            } else {
                ctx.say("❌ Submission not found.").await?;
            }
        }
        LoraxState::TieBreaker { options, votes, .. } => {
            if let Some(index) = options.iter().position(|name| name == &tree_name) {
                options.remove(index);
                votes.retain(|_, &mut vote_idx| vote_idx != index);
                for vote_idx in votes.values_mut() {
                    if *vote_idx > index {
                        *vote_idx -= 1;
                    }
                }
                settings.save()?;

                let msg = if let Some(reason) = reason {
                    format!(
                        "✅ Removed submission `{}` and updated votes (Reason: {})",
                        tree_name, reason
                    )
                } else {
                    format!("✅ Removed submission `{}` and updated votes", tree_name)
                };
                ctx.say(msg).await?;
            } else {
                ctx.say("❌ Submission not found.").await?;
            }
        }
        LoraxState::Idle => {
            ctx.say("No active event.").await?;
        }
    }

    Ok(())
}

#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn force_end(
    ctx: Context<'_>,
    #[description = "Reason for ending early"] reason: Option<String>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let data = ctx.data().clone();
    let serenity_ctx = ctx.serenity_context().clone();

    let settings = data.settings.write().await;
    let state = settings.guilds.get(&guild_id).unwrap().lorax_state.clone();
    drop(settings);

    match state {
        LoraxState::Submissions { .. } => {
            let msg = if let Some(reason) = reason {
                format!("🚨 Force ending submission phase! ({})", reason)
            } else {
                "🚨 Force ending submission phase!".to_string()
            };
            ctx.say(&msg).await?;

            info!("Force ending submission phase for guild {}", guild_id);
            start_voting(&serenity_ctx, &data, guild_id, 60).await?;
        }
        LoraxState::Voting { .. } | LoraxState::TieBreaker { .. } => {
            let msg = if let Some(reason) = reason {
                format!("🚨 Force ending voting phase! ({})", reason)
            } else {
                "🚨 Force ending voting phase!".to_string()
            };
            ctx.say(&msg).await?;

            info!("Force ending voting phase for guild {}", guild_id);
            announce_winner(&serenity_ctx.http, &data, guild_id).await?;
        }
        LoraxState::Idle => {
            ctx.say("No active event to end.").await?;
        }
    }

    Ok(())
}
