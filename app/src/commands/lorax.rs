use crate::metrics::MetricsClient;
use crate::settings::LoraxState;
use crate::{Context, Data, Error};
use chrono::Utc;
use poise::serenity_prelude::{
    self as serenity, futures, ButtonStyle, ChannelId, Color, ComponentInteraction,
    CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter,
    CreateInteractionResponseMessage, CreateSelectMenu, CreateSelectMenuKind,
    CreateSelectMenuOption, RoleId,
};
use poise::serenity_prelude::{futures::future::BoxFuture, FutureExt};
use poise::CreateReply;
use rand::rngs::{OsRng, StdRng};
use rand::{SeedableRng, Rng};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};
use rand::seq::SliceRandom;

/// Main command for Lorax events, with subcommands for managing the events.
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

/// Sets the channel for Lorax announcements.
#[poise::command(slash_command, required_permissions = "MANAGE_CHANNELS")]
pub async fn set_channel(
    ctx: Context<'_>,
    #[description = "Channel for node naming announcements"] channel: ChannelId,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let pool = Arc::clone(&ctx.data().pool);

    {
        let mut settings = ctx.data().settings.write().await;
        let mut guild_settings = settings.get_guild_settings(guild_id);
        guild_settings.lorax_channel = Some(channel);
        settings.set_guild_settings(guild_id, guild_settings);
        settings.save(&pool).await?;
    }

    ctx.say(format!(
        "Got it! I'll post all future Lorax announcements in <#{}>. 🌳",
        channel
    ))
    .await?;

    Ok(())
}

/// Sets the role to ping for Lorax events.
#[poise::command(slash_command, required_permissions = "MANAGE_ROLES")]
pub async fn set_role(
    ctx: Context<'_>,
    #[description = "Role to ping for node naming events"] role: RoleId,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let pool = Arc::clone(&ctx.data().pool);

    {
        let mut settings = ctx.data().settings.write().await;
        let mut guild_settings = settings.get_guild_settings(guild_id);
        guild_settings.lorax_role = Some(role);
        settings.set_guild_settings(guild_id, guild_settings);
        settings.save(&pool).await?;
    }

    ctx.say(format!(
        "Great! Members with the <@&{}> role will now get notifications about Lorax events. 🌿",
        role
    ))
    .await?;

    Ok(())
}

/// Starts a new Lorax event.
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn start(
    ctx: Context<'_>,
    #[description = "Location of the new node (e.g., 'US-East', 'EU-West')"] location: String,
    #[description = "Submission duration in minutes"] submission_duration: Option<u64>,
    #[description = "Voting duration in minutes"] voting_duration: Option<u64>,
    #[description = "Tiebreaker duration in minutes"] tiebreaker_duration: Option<u64>,
) -> Result<(), Error> {
    let submission_duration = submission_duration.unwrap_or(60);
    let voting_duration = voting_duration.unwrap_or(30);
    let tiebreaker_duration = tiebreaker_duration.unwrap_or(15);
    let guild_id = ctx.guild_id().unwrap();
    let data = ctx.data().clone();

    let mut settings = data.settings.write().await;
    let pool = data.pool;
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
            voting_duration,
            tiebreaker_duration,
        };
        settings.save(&pool).await?;

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
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    let pool = Arc::clone(&data.pool);
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
        voting_duration,
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
            settings.save(&pool).await?;
            return Ok(());
        }

        // Use voting_duration instead of hardcoded value
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

        // Create campaign thread
        let thread = channel_id.create_thread(
            ctx,
            serenity::CreateThread::new("🗳️ Tree Name Campaign Thread")
                .kind(serenity::ChannelType::PublicThread)
                .auto_archive_duration(serenity::AutoArchiveDuration::OneDay)
        )
        .await?;

        // Send initial thread message
        thread.id.send_message(
            ctx, 
            serenity::CreateMessage::new()
                .content("🌳 Welcome to the campaign thread! This is where submitters can advocate for their tree names and voters can discuss the options.")
        ).await?;

        let representatives = {
            let mut rng = StdRng::from_rng(OsRng)?;
            let mut submitter_list: Vec<_> = submissions.iter().collect();
            submitter_list.shuffle(&mut rng);
            submitter_list.into_iter().take(5).collect::<Vec<_>>()
        };

        // Send initial thread message with random representatives
        let reps = futures::future::join_all(representatives.iter().map(|(user_id, tree)| async move {
            let _name = get_submitter_name(ctx, **user_id).await;
            format!("🗣️ <@{}> representing `{}`", user_id, tree)
        })).await.join("\n");

        let campaign_msg = if !representatives.is_empty() {
            format!(
                "🌳 Welcome to the Tree Name Campaign Thread!\n\n\
                Here, submitters can advocate for their tree names and voters can discuss the options.\n\n\
                Some of our candidates speaking today:\n{}\n\n\
                May the best tree win! 🎉",
                reps
            )
        } else {
            "🌳 Welcome to the Tree Name Campaign Thread! This is where submitters can advocate for their tree names and voters can discuss the options.".to_string()
        };

        thread.id.send_message(
            ctx, 
            serenity::CreateMessage::new().content(campaign_msg)
        ).await?;

        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Voting {
            end_time,
            message_id: announcement_msg.id,
            thread_id: Some(thread.id),  // Store thread ID
            options,
            votes: HashMap::new(),

            submissions: submissions.clone(),
            location: location.clone(),
            tiebreaker_duration: *tiebreaker_duration,
        };
        settings.save(&pool).await?;
    }

    Ok(())
}

pub fn start_tiebreaker(
    http: Arc<serenity::Http>,
    data: Data,
    guild_id: serenity::GuildId,
    tied_options: Vec<(usize, String)>,
    location: String,
    round: u32,
    tiebreaker_duration: u64,
) -> BoxFuture<'static, Result<(), Error>> {
    async move {
        let mut settings = data.settings.write().await;
        let pool = Arc::clone(&data.pool);
        let guild = settings.guilds.get_mut(&guild_id).unwrap();

        let channel_id = guild.lorax_channel.unwrap();
        let role_id = guild.lorax_role.unwrap();

        let end_time = Utc::now().timestamp() + (tiebreaker_duration * 60) as i64;
        
        // Clone tied_options before consuming it
        let tied_options_clone = tied_options.clone();
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

        let announcement_msg = channel_id.say(http.clone(), announcement).await?;

        // Get existing thread ID if any
        let thread_id = match &guild.lorax_state {
            LoraxState::Voting { thread_id, .. } => *thread_id,
            LoraxState::TieBreaker { thread_id, .. } => *thread_id,
            _ => None,
        };

        guild.lorax_state = LoraxState::TieBreaker {
            end_time,
            message_id: announcement_msg.id,
            thread_id,  // Preserve the thread
            options: options.clone(),
            votes: HashMap::new(),
            location: location.clone(),
            round,
            tiebreaker_duration,
            submissions,
        };
        settings.save(&pool).await?;

        // If there's a thread, announce the tiebreaker there too
        if let Some(thread_id) = thread_id {
            let _ = thread_id.send_message(
                http,
                serenity::CreateMessage::new()
                    .content(format!(
                        "🎯 We're headed to tiebreaker round {}! The following names are tied:\n{}\n\nUse `/lorax vote` to help break the tie!",
                        round,
                        options
                            .iter()
                            .map(|name| format!("• `{}`", name))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ))
            ).await;
        }

        // Use OsRng for thread-safe random number generation
        let mut rng = StdRng::from_rng(OsRng)?;
        let _remove_idx = rng.gen_range(0..tied_options_clone.len());
        // Note: The actual elimination logic should go here if needed

        Ok(())
    }.boxed()
}

pub async fn announce_winner(
    http: &Arc<serenity::Http>,
    data: &Data,
    guild_id: serenity::GuildId,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    let pool = Arc::clone(&data.pool);
    let guild = settings.guilds.get_mut(&guild_id).unwrap();
    let channel_id = guild.lorax_channel.unwrap();
    let role_id = guild.lorax_role.unwrap();
    let state = guild.lorax_state.clone();

    // Track if we need to close a thread
    let mut thread_to_close = None;

    match state {
        LoraxState::Voting {
            thread_id,
            ..
        }
        | LoraxState::TieBreaker {
            thread_id,
            ..
        } => {
            thread_to_close = thread_id;
        }
        _ => {}
    }

    match state {
        LoraxState::Voting {
            options,
            votes,
            location,
            tiebreaker_duration,
            submissions,
            ..
        }
        | LoraxState::TieBreaker {
            options,
            votes,
            location,
            tiebreaker_duration,
            submissions,
            ..
        } => {
            let mut vote_counts: HashMap<usize, usize> = HashMap::new();
            for &choice in votes.values() {
                *vote_counts.entry(choice).or_insert(0) += 1;
            }

            if vote_counts.is_empty() || options.len() <= 1 {
                if let Some(winning_tree) = options.first() {
                    let submitter = submissions.iter()
                        .find(|(_, name)| *name == winning_tree)
                        .map(|(user_id, _)| *user_id)
                        .unwrap();

                    channel_id
                        .say(
                            http,
                            format!(
                                "Hey <@&{}>! 🎉 The winning tree name is **{}** (submitted by <@{}>)! This will be the name for our new **{}** node.\n\nThank you all for participating!",
                                role_id, winning_tree, submitter, location
                            ),
                        )
                        .await?;

                    guild.lorax_state = LoraxState::Idle;
                    settings.save(&pool).await?;
                } else {
                    channel_id
                        .say(
                            http,
                            "No valid tree names remain. The event has ended without a winner.",
                        )
                        .await?;

                    guild.lorax_state = LoraxState::Idle;
                    settings.save(&pool).await?;
                }
                return Ok(());
            }

            // Get sorted list of all entries by votes
            let mut options_with_votes: Vec<_> = options
                .iter()
                .enumerate()
                .map(|(idx, name)| {
                    let vote_count = vote_counts.get(&idx).unwrap_or(&0);
                    let submitter = submissions.iter()
                        .find(|(_, n)| *n == name)
                        .map(|(user_id, _)| *user_id)
                        .unwrap();
                    (name.clone(), *vote_count, submitter)
                })
                .collect();
            options_with_votes.sort_by(|a, b| b.1.cmp(&a.1));

            let max_votes = options_with_votes.first().map(|(_, votes, _)| *votes).unwrap_or(0);
            let tied_options: Vec<(usize, String)> = options
                .iter()
                .enumerate()
                .filter(|(i, _)| vote_counts.get(i).unwrap_or(&0) == &max_votes)
                .map(|(i, name)| (i, name.clone()))
                .collect();

            if tied_options.len() > 1 {
                let round = match guild.lorax_state {
                    LoraxState::TieBreaker { round, .. } => round + 1,
                    _ => 1,
                };

                let tied_names = tied_options.iter()
                    .map(|(_, name)| {
                        let submitter = submissions.iter()
                            .find(|(_, n)| *n == name)
                            .map(|(user_id, _)| *user_id)
                            .unwrap();
                        format!("`{}` (by <@{}>)", name, submitter)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                channel_id
                    .say(
                        http,
                        format!(
                            "Hey <@&{}>! We've got a tie between {}! Time for tiebreaker round {}!\n\nUse `/lorax vote` to break the tie!",
                            role_id, tied_names, round
                        ),
                    )
                    .await?;

                start_tiebreaker(
                    http.clone(),
                    data.clone(),
                    guild_id,
                    tied_options,
                    location.clone(),
                    round,
                    tiebreaker_duration,
                )
                .await?;
                return Ok(());
            }

            if let Some((winning_tree, winning_votes, submitter)) = options_with_votes.first() {
                let top_entries = if options_with_votes.len() > 1 {
                    let runners_up = options_with_votes[1..].iter()
                        .take(2)
                        .map(|(name, votes, user_id)| {
                            format!("`{}` by <@{}> ({} votes)", name, user_id, votes)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    
                    let remaining = if options_with_votes.len() > 3 {
                        format!("\n\n...and {} more entries!", options_with_votes.len() - 3)
                    } else {
                        String::new()
                    };

                    format!(
                        "\n\nTop entries:\n🥇 `{}` by <@{}> ({} votes)\n🥈 {}{}",
                        winning_tree, submitter, winning_votes, runners_up, remaining
                    )
                } else {
                    String::new()
                };

                channel_id
                    .say(
                        http,
                        format!(
                            "Hey <@&{}>! 🎉 The winning tree name is **{}**! This will be the name for our new **{}** node.{}",
                            role_id, winning_tree, location, top_entries
                        ),
                    )
                    .await?;

                guild.lorax_state = LoraxState::Idle;
                settings.save(&pool).await?;
            }
        }
        _ => {}
    }

    // After announcing winner or if no valid options remain, close the thread
    if let Some(thread_id) = thread_to_close {
        if let Err(e) = thread_id.edit_thread(
            http,
            serenity::EditThread::new().archived(true).locked(true),
        ).await {
            warn!("Failed to close campaign thread: {}", e);
        }
    }

    Ok(())
}

fn discord_timestamp(time: i64, style: TimestampStyle) -> String {
    format!("<t:{}:{}>", time, style.as_str())
}

enum TimestampStyle {
    ShortDateTime,
    Relative,
}

impl TimestampStyle {
    fn as_str(&self) -> &str {
        match self {
            TimestampStyle::ShortDateTime => "f",
            TimestampStyle::Relative => "R",
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

/// Cancels the current Lorax event.
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn cancel(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;
    let pool = Arc::clone(&ctx.data().pool);

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
            settings.save(&pool).await?;
            ctx.say("Alright, the current Lorax event has been cancelled and reset. 🛑")
                .await?;
        }
    }
    Ok(())
}

/// Adjusts the duration of the current Lorax event phase.
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn duration(
    ctx: Context<'_>,
    #[description = "Minutes to adjust (positive to extend, negative to reduce)"] minutes: i64,
) -> Result<(), Error> {
    duration_impl(ctx, minutes).await
}

async fn duration_impl(
    ctx: Context<'_>,
    minutes: i64,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;
    let pool = Arc::clone(&ctx.data().pool);
    let guild = settings.guilds.get_mut(&guild_id).unwrap();

    // Extract the needed data before match statement to avoid borrow conflicts
    let channel_id = guild.lorax_channel.unwrap();
    let thread_id = match &guild.lorax_state {
        LoraxState::Voting { thread_id, .. } | LoraxState::TieBreaker { thread_id, .. } => *thread_id,
        _ => None,
    };

    match &mut guild.lorax_state {
        LoraxState::Idle => {
            ctx.say("No active Lorax event to modify.").await?;
        }
        LoraxState::Submissions { end_time, message_id, .. }
        | LoraxState::Voting { end_time, message_id, .. }
        | LoraxState::TieBreaker { end_time, message_id, .. } => {
            let current_time = Utc::now().timestamp();
            let new_end = *end_time + (minutes * 60);

            if new_end <= current_time {
                ctx.say(
                    "Hmm, I can't set the end time to the past. Let's try a different amount. ⏳",
                )
                .await?;
                return Ok(());
            }

            let msg_id = *message_id;
            
            // Update the end time
            *end_time = new_end;

            // Save state changes
            settings.save(&pool).await?;
            drop(settings);  // Drop settings early to avoid borrowing conflicts

            // Edit the original announcement message
            if let Ok(mut msg) = channel_id.message(&ctx, msg_id).await {
                let new_content = msg.content.clone()
                    .replace(
                        msg.content.rsplit("<t:").next().unwrap_or("").rsplit_once(">").map(|(_, rest)| rest).unwrap_or(""),
                        &format!("\nVoting ends {}.", discord_timestamp(new_end, TimestampStyle::ShortDateTime))
                    );
                
                if let Err(e) = msg.edit(&ctx, serenity::EditMessage::new().content(new_content)).await {
                    warn!("Failed to edit announcement message: {}", e);
                }
            }

            // Send notification message
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

            // Also post in campaign thread if we're in voting phase
            if let Some(thread_id) = thread_id {
                let _ = thread_id.send_message(
                    &ctx,
                    serenity::CreateMessage::new().content(format!(
                        "⏰ Voting duration has been adjusted! New end time: {}",
                        discord_timestamp(new_end, TimestampStyle::ShortDateTime)
                    ))
                ).await;
            }

            ctx.say("Got it! The duration has been updated. 📅").await?;
        }
    }
    Ok(())
}

/// Submits a tree name suggestion.
#[poise::command(slash_command, ephemeral)]
pub async fn submit(
    ctx: Context<'_>,
    #[description = "Your tree name suggestion (lowercase letters only)"] name: String,
) -> Result<(), Error> {
    let pool = Arc::clone(&ctx.data().pool);
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

        let msg = match submissions.entry(user_id) {
            Entry::Vacant(vacancy) => {
                vacancy.insert(tree_name);
                "Awesome! I've updated your submission. Good luck! 🌲"
            }

            Entry::Occupied(mut occupied) => {
                occupied.insert(tree_name);
                "Thanks for your submission! Good luck! 🌴"
            }
        };

        settings.save(&pool).await?;
        ctx.say(msg).await?;
    } else {
        ctx.say("Submissions are not currently open.").await?;
    }

    Ok(())
}

/// Votes for a tree name.
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
        }
        | LoraxState::TieBreaker {
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
                    .default_selection(votes.get(&user_id) == Some(i))
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
    let pool = Arc::clone(&data.pool);
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
                    submissions,
                    end_time,
                    ..
                }
                | LoraxState::TieBreaker {
                    votes,
                    options,
                    submissions,
                    end_time,
                    ..
                } = &mut guild.lorax_state
                {
                    if Utc::now().timestamp() > *end_time {
                        "Voting period has ended.".to_string()
                    } else if let Some(selected_tree) = options.get(choice).cloned() {
                        if let Some((submitter_id, _)) = submissions
                            .iter()
                            .find(|(_, tree)| **tree == selected_tree)
                        {
                            if *submitter_id == user_id {
                                "You can't vote for your own submission!".to_string()
                            } else {
                                votes.insert(user_id, choice);
                                settings.save(&pool).await?;
                                format!("Your vote for `{}` has been recorded!", selected_tree)
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

/// Lists the current submissions or voting status.
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
                if !is_admin {
                    ctx.say(
                        "Only administrators can view submissions during the submission phase.",
                    )
                    .await?;
                    return Ok(());
                }

                let submission_list =
                    futures::future::join_all(submissions.iter().map(|(user_id, name)| {
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
                ..
            }
            | LoraxState::TieBreaker {
                options,
                votes,
                end_time,
                ..
            } => {
                if !is_admin {
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

/// Removes a tree name submission.
#[poise::command(slash_command, required_permissions = "MANAGE_MESSAGES")]
pub async fn remove(
    ctx: Context<'_>,
    #[description = "Tree name to remove"] tree_name: String,
    #[description = "Reason for removal"] reason: Option<String>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let mut settings = ctx.data().settings.write().await;
    let pool = Arc::clone(&ctx.data().pool);
    let guild = settings.guilds.get_mut(&guild_id).unwrap();

    match &mut guild.lorax_state {
        LoraxState::Submissions { submissions, .. } => {
            if let Some((user_id, _)) = submissions.iter().find(|(_, name)| name == &&tree_name) {
                let user_id = *user_id;
                submissions.remove(&user_id);
                settings.save(&pool).await?;

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
                settings.save(&pool).await?;

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
                settings.save(&pool).await?;

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

/// Forces the current Lorax event phase to end early.
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
            start_voting(&serenity_ctx, &data, guild_id).await?;
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

/// Displays the current status of the Lorax event.
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
