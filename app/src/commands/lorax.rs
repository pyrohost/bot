use crate::metrics::MetricsClient;
use crate::settings::LoraxState;
use crate::{Context, Data, Error};
use chrono::{Duration, Utc};
use poise::serenity_prelude::{
    self as serenity, ButtonStyle, ChannelId, Color, ComponentInteraction, CreateActionRow,
    CreateButton, CreateEmbed, CreateEmbedFooter, CreateInteractionResponseMessage,
    CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption, RoleId, UserId,
};
use poise::CreateReply;
use std::collections::HashMap;

/// Lorax - Tree-themed Node Naming System
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
        "setup",
        "remove",
        "force_end",
    )
)]
pub async fn lorax(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Set the Lorax announcement channel
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

/// Set the Lorax role to ping
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
) -> Result<(), Error> {
    let submission_duration = submission_duration.unwrap_or(60);
    let voting_duration = voting_duration.unwrap_or(60);
    let guild_id = ctx.guild_id().unwrap();

    let mut settings = ctx.data().settings.write().await;
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

        // Announce the start of submissions
        let channel_id = guild_settings.lorax_channel.unwrap();
        let announcement_msg = channel_id.say(&ctx, announcement).await?;

        // Update LoraxState to Submissions with location
        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Submissions {
            end_time,
            message_id: announcement_msg.id,
            submissions: HashMap::new(),
            location,
        };
        settings.save()?;

        // Schedule transition to voting
        let data = ctx.data().clone();
        let ctx_clone = ctx.serenity_context().clone();
        tokio::spawn(async move {
            let seconds = submission_duration as i64 * 60;
            tokio::time::sleep(Duration::seconds(seconds).to_std().unwrap()).await;

            if let Err(e) = start_voting(&ctx_clone, &data, guild_id, voting_duration).await {
                tracing::error!("Failed to start voting: {}", e);
            }
        });

        ctx.say("🎉 Lorax event started! Submissions are now open.").await?;
    } else {
        ctx.say("⚠️ A Lorax event is already in progress.").await?;
    }

    Ok(())
}

// Helper function to start voting phase
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
        ..
    } = state
    {
        if submissions.is_empty() {
            // No submissions, end the event
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
            Voting ends {}. Don't miss out!",
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
            // We need access to the submissions in the Voting.
            submissions: submissions.clone(),
            location: location.clone(),
        };
        settings.save()?;

        // Schedule end of voting
        let data_clone = data.clone();
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            let seconds = voting_duration as i64 * 60;
            tokio::time::sleep(Duration::seconds(seconds).to_std().unwrap()).await;
            if let Err(e) = announce_winner(&ctx_clone, &data_clone, guild_id).await {
                tracing::error!("Failed to announce winner: {}", e);
            }
        });
    }

    Ok(())
}

// Helper function to announce the winner
pub async fn announce_winner(
    ctx: &serenity::Context,
    data: &Data,
    guild_id: serenity::GuildId,
) -> Result<(), Error> {
    let mut settings = data.settings.write().await;
    // Remove unused guild_settings variable and use direct access
    let guild = settings.guilds.get(&guild_id).unwrap();

    let channel_id = guild.lorax_channel.unwrap();

    if let LoraxState::Voting {
        options,
        votes,
        submissions,
        location,
        ..
    } = &guild.lorax_state
    {
        // Tally votes
        let mut vote_counts = HashMap::new();
        for &choice in votes.values() {
            *vote_counts.entry(choice).or_insert(0) += 1;
        }

        let total_votes = votes.len();

        let announcement_prefix = format!("🎉 Our **{}** node has a new name!\n\n", location);

        if total_votes == 0 && options.len() == 1 {
            // Only one submission, declare it the winner
            let winning_tree = &options[0];
            let winner_mention = submissions
                .iter()
                .find(|(_, tree)| *tree == winning_tree)
                .map_or("Unknown User".to_string(), |(user_id, _)| {
                    format!("<@{}>", user_id)
                });

            let announcement = format!(
                "{}Say hello to our newest node: `{}`! A big thank you to {} for the fantastic suggestion! 🌟",
                announcement_prefix, winning_tree, winner_mention
            );
            channel_id.say(ctx, announcement).await?;
        } else if let Some((&winning_option, _count)) =
            vote_counts.iter().max_by_key(|&(_, count)| count)
        {
            let winning_tree = &options[winning_option];

            let winner_mention = submissions
                .iter()
                .find(|(_, tree)| *tree == winning_tree)
                .map_or("Unknown User".to_string(), |(user_id, _)| {
                    format!("<@{}>", user_id)
                });

            let _role_id = guild.lorax_role.unwrap();
            let vote_distribution = options
                .iter()
                .enumerate()
                .map(|(i, name)| {
                    let votes = *vote_counts.get(&i).unwrap_or(&0);
                    let percentage = if total_votes > 0 {
                        (votes * 100) / total_votes
                    } else {
                        0
                    };
                    format!(
                        "{} {} - {} votes ({}%)",
                        if i == winning_option { "👑" } else { "🌳" },
                        *name,
                        votes,
                        percentage
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let mut sorted_votes: Vec<_> = vote_counts.iter().collect();
            sorted_votes.sort_by_key(|&(_, count)| std::cmp::Reverse(*count));

            let top_three = sorted_votes
                .iter()
                .take(3)
                .enumerate()
                .map(|(i, (&option_idx, &count))| {
                    let tree_name = &options[option_idx];
                    let submitter = submissions
                        .iter()
                        .find(|(_, name)| name == &tree_name)
                        .map(|(user_id, _)| format!("<@{}>", user_id))
                        .unwrap_or_else(|| "Unknown User".to_string());
                    format!(
                        "{} {} - {} votes (submitted by {})",
                        match i {
                            0 => "🥇",
                            1 => "🥈",
                            2 => "🥉",
                            _ => "🌳",
                        },
                        tree_name,
                        count,
                        submitter
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let announcement = format!(
                "{}And the winning name is... `{}`! Congratulations, {}! 🎊\n\n\
                **Final Results:**\n{}\n\nThanks to everyone who participated!",
                announcement_prefix,
                winning_tree,
                winner_mention,
                top_three
            );
            channel_id.say(ctx, announcement).await?;
        } else {
            // NOTE: If we have no votes but only 1 tree, maybe we should just crown that the winner?
            // Especially if we aren't able to vote for our own submission.

            let winning_tree = &options[0];
            let winner_mention = submissions
                .iter()
                .find(|(_, tree)| *tree == winning_tree)
                .map_or("Unknown User".to_string(), |(user_id, _)| {
                    format!("<@{}>", user_id)
                });

            let announcement = format!(
                "{}{}",
                announcement_prefix,
                format!(
                    "**Winner by Default:** {}\nSubmitted by {}\n\n_Thank you for participating!_",
                    winning_tree, winner_mention
                )
            );
            channel_id.say(ctx, announcement).await?;
        }

        // Reset Lorax state
        settings.guilds.get_mut(&guild_id).unwrap().lorax_state = LoraxState::Idle;
        settings.save()?;
    }

    Ok(())
}

// Helper function to format Discord timestamps
fn discord_timestamp(time: i64, style: TimestampStyle) -> String {
    format!("<t:{}:{}>", time, style.as_str())
}

enum TimestampStyle {
    ShortTime,     // t - 9:41 PM
    LongTime,      // T - 9:41:30 PM
    ShortDate,     // d - 06/09/2023
    LongDate,      // D - June 9, 2023
    ShortDateTime, // f - June 9, 2023 9:41 PM
    LongDateTime,  // F - Friday, June 9, 2023 9:41 PM
    Relative,      // R - 2 months ago
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

// Add this near the top of the file with other constants/statics
const RESERVED_NAMES: &[&str] = &[
    "sakura", // Japan region
    "cherry", // Japan region
    "bamboo", // Reserved for future APAC
    "maple",  // Reserved for Canada
    "pine",   // Reserved for Nordic
    "palm",   // Reserved for tropical regions
    "cedar",  // Reserved for Middle East
];

// Helper function to validate tree name
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

/// Cancel ongoing Lorax event
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
            ctx.say("Alright, the current Lorax event has been cancelled and reset. 🛑").await?;
        }
    }
    Ok(())
}

/// Modify current phase duration (extend or retract)
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
        LoraxState::Submissions { end_time, .. } | LoraxState::Voting { end_time, .. } => {
            let current_time = Utc::now().timestamp();
            let new_end = *end_time + (minutes * 60);

            // Ensure we don't set end time in the past
            if new_end <= current_time {
                ctx.say("Hmm, I can't set the end time to the past. Let's try a different amount. ⏳").await?;
                return Ok(());
            }

            *end_time = new_end;

            // Clone necessary values before dropping mutable borrow
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

/// Submit a tree name suggestion for the current event
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

    // Check for existing tree names using metrics API
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
        // Check if name is already submitted by someone else
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

/// Vote for a tree name suggestion
#[poise::command(slash_command, ephemeral)]
pub async fn vote(
    ctx: Context<'_>,
    #[description = "Page number"] page: Option<u32>,
    #[description = "Search for specific names"] search: Option<String>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
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

            // Filter options based on search term
            let filtered_options: Vec<_> = if let Some(ref search) = search {
                options
                    .iter()
                    .enumerate()
                    .filter(|(_, name)| name.contains(&search.to_lowercase()))
                    .collect()
            } else {
                options.iter().enumerate().collect()
            };

            // Calculate pagination
            let total_pages = (filtered_options.len() + ITEMS_PER_PAGE - 1) / ITEMS_PER_PAGE;
            let start_idx = (page as usize - 1) * ITEMS_PER_PAGE;
            let end_idx = start_idx + ITEMS_PER_PAGE;
            let page_options = &filtered_options
                [start_idx.min(filtered_options.len())..end_idx.min(filtered_options.len())];

            let mut components = Vec::new();

            // Create select menu for voting
            let select_options = page_options
                .iter()
                .map(|(i, name)| {
                    CreateSelectMenuOption::new(
                        format!(
                            "{} - Submitted by {}",
                            name,
                            get_submitter_name(submissions, name)
                        ),
                        i.to_string(),
                    )
                    .default_selection(votes.get(&user_id) == Some(&i))
                })
                .collect::<Vec<_>>();

            if !select_options.is_empty() {
                components.push(CreateActionRow::SelectMenu(
                    CreateSelectMenu::new(
                        "vote_select",
                        CreateSelectMenuKind::String {
                            options: select_options,
                        },
                    )
                    .placeholder("Select a tree name to vote for"),
                ));
            }

            // Add navigation buttons if needed
            if total_pages > 1 {
                let mut nav_buttons = Vec::new();
                if page > 1 {
                    nav_buttons.push(
                        CreateButton::new(format!("page_{}", page - 1))
                            .label("Previous")
                            .style(ButtonStyle::Secondary),
                    );
                }
                if page < total_pages as u32 {
                    nav_buttons.push(
                        CreateButton::new(format!("page_{}", page + 1))
                            .label("Next")
                            .style(ButtonStyle::Secondary),
                    );
                }
                if !nav_buttons.is_empty() {
                    components.push(CreateActionRow::Buttons(nav_buttons));
                }
            }

            let _current_vote = votes.get(&user_id).map(|&idx| &options[idx]);
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

// Helper function to get submitter's name
fn get_submitter_name(submissions: &HashMap<UserId, String>, tree_name: &str) -> String {
    submissions
        .iter()
        .find(|(_, name)| name.as_str() == tree_name)
        .map(|(user_id, _)| format!("<@{}>", user_id))
        .unwrap_or_else(|| "Unknown".to_string())
}

/// View the current Lorax event status
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
    };

    ctx.say(status_msg).await?;
    Ok(())
}

/// View current submissions (Administrators only)
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD", ephemeral)]
pub async fn list(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;

    // Ensure only administrators can view submissions
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

                let submission_list = submissions
                    .iter()
                    .map(|(user_id, name)| format!("• `{}` (by <@{}>)", name, user_id))
                    .collect::<Vec<_>>()
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
                options, end_time, ..
            } => {
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
            }
            LoraxState::Idle => {
                ctx.say("No active tree naming event.").await?;
            }
        }
    }

    Ok(())
}

/// Setup the Lorax system for this server
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn setup(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let settings = ctx.data().settings.read().await;
    let guild_settings = settings.get_guild_settings(guild_id);

    let mut setup_status = vec![];

    // Check channel setup
    setup_status.push(format!(
        "📢 Announcement Channel: {}",
        if let Some(channel) = guild_settings.lorax_channel {
            format!("Set to <#{}>", channel)
        } else {
            "❌ Not set - Use `/lorax set_channel`".to_string()
        }
    ));

    // Check role setup
    setup_status.push(format!(
        "👥 Ping Role: {}",
        if let Some(role) = guild_settings.lorax_role {
            format!("Set to <@&{}>", role)
        } else {
            "❌ Not set - Use `/lorax set_role`".to_string()
        }
    ));

    // Current state
    setup_status.push(format!(
        "📋 Current State: {}",
        match guild_settings.lorax_state {
            LoraxState::Idle => "Ready for new event",
            LoraxState::Submissions { .. } => "Submission phase active",
            LoraxState::Voting { .. } => "Voting phase active",
        }
    ));

    ctx.send(
        CreateReply::default().embed(
            CreateEmbed::default()
                .title("🌳 Lorax System Setup")
                .description(setup_status.join("\n\n"))
                .footer(CreateEmbedFooter::new(
                    if guild_settings.lorax_channel.is_some() && guild_settings.lorax_role.is_some()
                    {
                        "✅ All set! Use `/lorax start` to kick off a new naming event."
                    } else {
                        "⚠️ Setup incomplete. Configure the missing options to start Lorax events."
                    },
                ))
                .color(
                    if guild_settings.lorax_channel.is_some() && guild_settings.lorax_role.is_some()
                    {
                        Color::from_rgb(67, 160, 71)
                    } else {
                        Color::from_rgb(244, 67, 54)
                    },
                ),
        ),
    )
    .await?;

    Ok(())
}

pub async fn handle_button(
    ctx: &serenity::Context,
    component: &ComponentInteraction,
    data: Data,
) -> Result<(), Error> {
    // Handle page navigation
    if component.data.custom_id.starts_with("page_") {
        // Handle pagination - implement if needed
        return Ok(());
    }

    // Handle vote selection
    if component.data.custom_id == "vote_select" {
        let choice = match &component.data.kind {
            serenity::ComponentInteractionDataKind::StringSelect { values } => {
                if let Some(value) = values.first() {
                    value
                        .parse::<usize>()
                        .map_err(|_| "Invalid vote selection")?
                } else {
                    return Ok(());
                }
            }
            _ => return Ok(()),
        };

        let guild_id = component.guild_id.unwrap();
        let user_id = component.user.id;

        let mut settings = data.settings.write().await;

        // Validate vote and check end time
        if let Some(guild) = settings.guilds.get(&guild_id) {
            if let LoraxState::Voting {
                options,
                end_time,
                submissions,
                ..
            } = &guild.lorax_state
            {
                if Utc::now().timestamp() > *end_time {
                    let builder = serenity::CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::default()
                            .ephemeral(true)
                            .content("Voting period has ended."),
                    );

                    component.create_response(ctx, builder).await?;
                    return Ok(());
                }

                if let Some(selected_tree) = options.get(choice) {
                    // Check if user is voting for their own submission
                    if let Some(submitter_id) = submissions
                        .iter()
                        .find(|(_, tree)| *tree == selected_tree)
                        .map(|(id, _)| *id)
                    {
                        if submitter_id == user_id {
                            let builder = serenity::CreateInteractionResponse::Message(
                                CreateInteractionResponseMessage::default()
                                    .ephemeral(true)
                                    .content("You can't vote for your own submission! Please choose a different name."),
                            );

                            component.create_response(ctx, builder).await?;
                            return Ok(());
                        }
                    }
                }
            }
        }

        // Record the vote
        if let Some(guild) = settings.guilds.get_mut(&guild_id) {
            if let LoraxState::Voting { votes, options, .. } = &mut guild.lorax_state {
                let previous_vote = votes.insert(user_id, choice);

                let tree_name = &options[choice];
                let response = if previous_vote.is_some() {
                    format!("👍 Got it! You've changed your vote to `{}`.", tree_name)
                } else {
                    format!("👍 Thanks! You've voted for `{}`.", tree_name)
                };

                let builder = serenity::CreateInteractionResponse::Message(
                    CreateInteractionResponseMessage::default()
                        .ephemeral(true)
                        .content(response),
                );

                component.create_response(ctx, builder).await?;

                settings.save()?;
            }
        }
    }

    Ok(())
}

/// Remove a submission from the current event
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
                    format!("✅ Removed submission `{}` (Reason: {}).", tree_name, reason)
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
                // Remove any votes for this option
                votes.retain(|_, &mut vote_idx| vote_idx != index);
                // Adjust remaining vote indices
                for vote_idx in votes.values_mut() {
                    if *vote_idx > index {
                        *vote_idx -= 1;
                    }
                }
                // Remove from submissions tracking
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
        LoraxState::Idle => {
            ctx.say("No active event.").await?;
        }
    }

    Ok(())
}

/// Force end the current phase immediately
#[poise::command(slash_command, required_permissions = "MANAGE_GUILD")]
pub async fn force_end(
    ctx: Context<'_>,
    #[description = "Reason for ending early"] reason: Option<String>,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().unwrap();
    let data = ctx.data().clone();
    let serenity_ctx = ctx.serenity_context().clone();

    let mut settings = data.settings.write().await;
    match &settings.guilds.get(&guild_id).unwrap().lorax_state {
        LoraxState::Submissions { .. } => {
            drop(settings); // Drop the lock before the next async call

            let msg = if let Some(reason) = reason {
                format!("🚨 Force ending the current phase! ({})", reason)
            } else {
                "🚨 Force ending the current phase!".to_string()
            };
            ctx.say(&msg).await?;

            start_voting(&serenity_ctx, &data, guild_id, 60).await?;
        }
        LoraxState::Voting { .. } => {
            drop(settings); // Drop the lock before the next async call

            let msg = if let Some(reason) = reason {
                format!("🚨 Force ending the current phase! ({})", reason)
            } else {
                "🚨 Force ending the current phase!".to_string()
            };
            ctx.say(&msg).await?;

            announce_winner(&serenity_ctx, &data, guild_id).await?;
        }
        LoraxState::Idle => {
            ctx.say("No active event to end.").await?;
        }
    }

    Ok(())
}
