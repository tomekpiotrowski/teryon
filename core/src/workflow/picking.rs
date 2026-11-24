//! Which agent and model work in a stage, picked from the stage's settings and what each provider has left.
//!
//! Different work suits different models, and each subscription runs out at
//! its own pace. A stage therefore asks for a strength (each provider names
//! its strong, medium and fast model) and for a provider in one of three
//! ways: any provider with usage left, one preferred provider with others to
//! fall back on, or only one provider, waiting when it has run out.
//!
//! A pick sticks to the thread for that stage, so a round of fixes after a
//! review goes back to the same agent rather than handing the thread to
//! another program each time. "Prefer" is the exception: the preferred
//! provider is taken again as soon as it has usage left.

use std::env::{split_paths, var_os};
use std::path::Path;

use chrono::{DateTime, Local, Utc};
use thiserror::Error;

use crate::agents::Agent;
use crate::settings::{AgentSettings, ProviderChoice, StageSetting, Strength};
use crate::usage::Limits;

/// When a provider can work again: now, or once a usage limit resets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Left {
    Yes,
    /// Used up, until the given moment when the program said.
    UsedUp(Option<DateTime<Utc>>),
}

/// Whether a provider has usage left, from what its program last said about its limits.
///
/// A provider never read, or whose reading failed, counts as having usage
/// left: Teryon knows no better, and its next turn will say if it is wrong.
/// A limit that has reset since counts as available again.
pub fn usage_left(limits: Option<&Limits>, now: DateTime<Utc>) -> Left {
    // A failed reading keeps the windows read before it, which may be long out of date.
    let Some(limits) = limits.filter(|limits| !limits.failed) else {
        return Left::Yes;
    };
    let reached = limits
        .windows
        .iter()
        .filter(|window| window.reached)
        .filter(|window| window.resets_at.is_none_or(|resets| resets > now));
    let mut until = None;
    for window in reached {
        match window.resets_at {
            None => return Left::UsedUp(None),
            Some(resets) => until = until.max(Some(resets)),
        }
    }
    match until {
        Some(until) => Left::UsedUp(Some(until)),
        None => Left::Yes,
    }
}

/// Whether an agent's program can be found on this machine.
///
/// A program named with a folder, as in tests and odd installs, is looked
/// for where it says; a bare name on `PATH`, as a shell would.
pub fn installed(program: &str) -> bool {
    if program.contains('/') || program.contains('\\') {
        return Path::new(program).is_file();
    }
    var_os("PATH").is_some_and(|path| {
        split_paths(&path).any(|folder| {
            folder.join(program).is_file() || folder.join(format!("{program}.exe")).is_file()
        })
    })
}

/// What picking knows about the thread and the providers.
pub struct Situation<'a> {
    /// The stage's label, for messages.
    pub stage: &'a str,
    /// Whether the pick is for a review, which never goes to the change's author.
    pub author: Option<Agent>,
    /// The provider picked for this stage before, which is kept while it has usage left.
    pub earlier: Option<Agent>,
    /// A provider that just ran out in this stage, left out whatever its limits say.
    pub ran_out: Option<Agent>,
    /// The providers whose programs are on this machine.
    pub installed: &'a [Agent],
    /// How much each provider has left.
    pub left: &'a dyn Fn(Agent) -> Left,
}

/// The agent and model a stage gets, and why, for the line in the thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub agent: Agent,
    /// Nothing runs the program's default model.
    pub model: Option<String>,
    /// How the provider was picked, such as "picked at random".
    pub why: String,
}

/// Why nobody can work in a stage. The text is meant for the person.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PickError {
    #[error("{agent}'s usage is used up{until}, and {stage} allows only {agent}.")]
    UsedUp {
        agent: &'static str,
        until: String,
        stage: String,
    },
    #[error("{agent} wrote this change, so it cannot review it, and {stage} allows only {agent}.")]
    OnlyAuthor { agent: &'static str, stage: String },
    #[error("{agent} isn't installed on this machine, and {stage} allows only {agent}.")]
    OnlyMissing { agent: &'static str, stage: String },
    #[error("No provider with usage left can work in {stage}{why}.")]
    NoneLeft { stage: String, why: String },
}

/// Picks who works in a stage. `random` gives a number below the one it is given.
pub fn pick(
    setting: &StageSetting,
    agents: &AgentSettings,
    situation: &Situation<'_>,
    random: &mut dyn FnMut(usize) -> usize,
) -> Result<Pick, PickError> {
    let choice = setting.provider.unwrap_or(ProviderChoice::Any);
    let strength = setting.strength.unwrap_or(Strength::Medium);
    let usable = |agent: Agent| -> Result<(), Unusable> {
        if situation.author == Some(agent) {
            return Err(Unusable::Author);
        }
        if !situation.installed.contains(&agent) {
            return Err(Unusable::Missing);
        }
        if situation.ran_out == Some(agent) {
            return Err(Unusable::UsedUp(None));
        }
        match (situation.left)(agent) {
            Left::Yes => Ok(()),
            Left::UsedUp(until) => Err(Unusable::UsedUp(until)),
        }
    };
    let (agent, why) = match choice {
        ProviderChoice::Only(agent) => match usable(agent) {
            Ok(()) => (agent, format!("only {}", agent.label())),
            Err(Unusable::Author) => {
                return Err(PickError::OnlyAuthor {
                    agent: agent.label(),
                    stage: situation.stage.to_string(),
                })
            }
            Err(Unusable::Missing) => {
                return Err(PickError::OnlyMissing {
                    agent: agent.label(),
                    stage: situation.stage.to_string(),
                })
            }
            Err(Unusable::UsedUp(until)) => {
                return Err(PickError::UsedUp {
                    agent: agent.label(),
                    until: until_text(until),
                    stage: situation.stage.to_string(),
                })
            }
        },
        ProviderChoice::Prefer(preferred) => match usable(preferred) {
            Ok(()) => (preferred, format!("preferred {}", preferred.label())),
            Err(unusable) => {
                let (agent, how) = fallback(setting, situation, &usable, random, Some(preferred))?;
                let reason = unusable.reason(preferred);
                (
                    agent,
                    format!(
                        "{reason}, so {how} instead of preferred {}",
                        preferred.label()
                    ),
                )
            }
        },
        ProviderChoice::Any => fallback(setting, situation, &usable, random, None)?,
    };
    let model = agents.model_for(agent, strength);
    let why = match &model {
        Some(_) => why,
        None => format!(
            "{why}; {} has no {} model set, so it runs its default",
            agent.label(),
            strength.name()
        ),
    };
    Ok(Pick { agent, model, why })
}

/// Why a provider can't be picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unusable {
    Author,
    Missing,
    UsedUp(Option<DateTime<Utc>>),
}

impl Unusable {
    fn reason(self, agent: Agent) -> String {
        match self {
            Self::Author => format!("{} wrote the change", agent.label()),
            Self::Missing => format!("{} isn't installed", agent.label()),
            Self::UsedUp(until) => format!("{} is used up{}", agent.label(), until_text(until)),
        }
    }
}

/// Another provider: the earlier pick when it can still work, otherwise one at random.
fn fallback(
    setting: &StageSetting,
    situation: &Situation<'_>,
    usable: &dyn Fn(Agent) -> Result<(), Unusable>,
    random: &mut dyn FnMut(usize) -> usize,
    preferred: Option<Agent>,
) -> Result<(Agent, String), PickError> {
    let allowed: Vec<Agent> = match &setting.among {
        Some(among) => among.iter().map(|name| name.0).collect(),
        None => Agent::ALL.to_vec(),
    };
    let candidates: Vec<Agent> = allowed
        .iter()
        .copied()
        .filter(|agent| Some(*agent) != preferred && usable(*agent).is_ok())
        .collect();
    if let Some(earlier) = situation.earlier.filter(|agent| candidates.contains(agent)) {
        return Ok((earlier, format!("{} as before", earlier.label())));
    }
    if candidates.is_empty() {
        let reasons: Vec<String> = allowed
            .iter()
            .filter_map(|agent| usable(*agent).err().map(|unusable| unusable.reason(*agent)))
            .collect();
        return Err(PickError::NoneLeft {
            stage: situation.stage.to_string(),
            why: match reasons.is_empty() {
                true => String::new(),
                false => format!(" ({})", reasons.join("; ")),
            },
        });
    }
    let agent = candidates[random(candidates.len()).min(candidates.len() - 1)];
    Ok((agent, format!("{} picked at random", agent.label())))
}

/// " until 14:00" in local time, or nothing when the program didn't say.
fn until_text(until: Option<DateTime<Utc>>) -> String {
    until.map_or_else(String::new, |until| {
        format!(" until {}", until.with_timezone(&Local).format("%H:%M"))
    })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use lets_expect::lets_expect;

    use super::*;
    use crate::settings::AgentName;
    use crate::usage::Window;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap()
    }

    fn window(reached: bool, resets_at: Option<DateTime<Utc>>) -> Window {
        Window {
            name: "five_hour".to_string(),
            label: "5 hours".to_string(),
            minutes: Some(300),
            used: if reached { 100.0 } else { 40.0 },
            resets_at,
            reached,
        }
    }

    fn limits(windows: Vec<Window>) -> Limits {
        Limits {
            agent: "claude".to_string(),
            plan: None,
            windows,
            taken_at: now(),
            failed: false,
            advice: None,
        }
    }

    fn failed_limits() -> Limits {
        Limits {
            failed: true,
            ..limits(Vec::new())
        }
    }

    fn failed_after_running_out() -> Limits {
        Limits {
            failed: true,
            ..limits(vec![window(true, None)])
        }
    }

    fn later(hours: i64) -> Option<DateTime<Utc>> {
        Some(now() + Duration::hours(hours))
    }

    fn setting(provider: Option<ProviderChoice>, strength: Option<Strength>) -> StageSetting {
        StageSetting {
            provider,
            strength,
            ..StageSetting::default()
        }
    }

    fn among(provider: ProviderChoice, agents: &[Agent]) -> StageSetting {
        StageSetting {
            provider: Some(provider),
            among: Some(agents.iter().copied().map(AgentName).collect()),
            ..StageSetting::default()
        }
    }

    /// Claude with its built-in strengths, Codex with a strong model only, Grok and Gemini with none.
    fn agents() -> AgentSettings {
        let mut agents = AgentSettings::default();
        agents.codex.strong = Some("gpt-6-astra".to_string());
        agents
    }

    fn situation<'a>(
        author: Option<Agent>,
        earlier: Option<Agent>,
        ran_out: Option<Agent>,
        installed: &'a [Agent],
        left: &'a dyn Fn(Agent) -> Left,
    ) -> Situation<'a> {
        Situation {
            stage: "Implementing",
            author,
            earlier,
            ran_out,
            installed,
            left,
        }
    }

    fn all_left(_: Agent) -> Left {
        Left::Yes
    }

    fn claude_used_up(agent: Agent) -> Left {
        match agent {
            Agent::Claude => Left::UsedUp(None),
            _ => Left::Yes,
        }
    }

    fn only_grok_left(agent: Agent) -> Left {
        match agent {
            Agent::Grok => Left::Yes,
            _ => Left::UsedUp(None),
        }
    }

    fn none_left(_: Agent) -> Left {
        Left::UsedUp(None)
    }

    /// Picks with a random choice that always takes the last candidate, so tests can tell it from the first.
    fn picked(
        setting: StageSetting,
        author: Option<Agent>,
        earlier: Option<Agent>,
        ran_out: Option<Agent>,
        installed: &[Agent],
        left: fn(Agent) -> Left,
    ) -> Result<Found, PickError> {
        let situation = situation(author, earlier, ran_out, installed, &left);
        pick(&setting, &agents(), &situation, &mut |count| count - 1)
            .map(|pick| (pick.agent, pick.model, pick.why))
    }

    const ALL: &[Agent] = &Agent::ALL;

    type Found = (Agent, Option<String>, String);

    fn agent_of(found: Found) -> Agent {
        found.0
    }

    fn why_of(found: Found) -> String {
        found.2
    }

    fn used_up(agent: &'static str) -> PickError {
        PickError::UsedUp {
            agent,
            until: String::new(),
            stage: "Implementing".to_string(),
        }
    }

    fn only_author(agent: &'static str) -> PickError {
        PickError::OnlyAuthor {
            agent,
            stage: "Implementing".to_string(),
        }
    }

    fn only_missing(agent: &'static str) -> PickError {
        PickError::OnlyMissing {
            agent,
            stage: "Implementing".to_string(),
        }
    }

    fn none_left_because(why: &str) -> PickError {
        PickError::NoneLeft {
            stage: "Implementing".to_string(),
            why: why.to_string(),
        }
    }

    lets_expect! {
        expect(usage_left(limits.as_ref(), now())) as usage_left_of_a_provider {
            when(limits = None) as a_provider_never_read to have_usage_left { equal(Left::Yes) }
            when(limits = Some(limits(vec![window(false, later(2))]))) as a_window_not_reached to have_usage_left { equal(Left::Yes) }
            when(limits = Some(limits(vec![window(true, later(2))]))) as a_window_reached_that_resets_later to be_used_up_until_then { equal(Left::UsedUp(later(2))) }
            when(limits = Some(limits(vec![window(true, later(-1))]))) as a_window_reached_that_has_reset to have_usage_left { equal(Left::Yes) }
            when(limits = Some(limits(vec![window(true, None)]))) as a_window_reached_without_a_reset_time to be_used_up { equal(Left::UsedUp(None)) }
            when(limits = Some(limits(vec![window(false, later(1)), window(true, later(30))]))) as one_of_several_windows_reached to be_used_up_until_it_resets { equal(Left::UsedUp(later(30))) }
            when(limits = Some(limits(vec![window(true, later(1)), window(true, later(30))]))) as two_windows_reached to wait_for_the_later { equal(Left::UsedUp(later(30))) }
            when(limits = Some(failed_limits())) as a_failed_reading to have_usage_left { equal(Left::Yes) }
            when(limits = Some(failed_after_running_out())) as a_failed_reading_over_a_window_used_up_before to have_usage_left { equal(Left::Yes) }
        }

        expect(installed(program)) as whether_a_program_is_installed {
            when(program = "tests/samples/fake-claude.sh") as a_program_named_with_its_folder to be_found { be_true }
            when(program = "tests/samples/nothing-here.sh") as a_missing_program_with_a_folder to be_missing { be_false }
            when(program = "sh") as a_program_on_the_path to be_found { be_true }
            when(program = "no-such-program-anywhere") as a_program_nowhere to be_missing { be_false }
        }

        expect(picked(setting, author, earlier, ran_out, installed, left)) as picking {
            let author = None;
            let earlier = None;
            let ran_out = None;
            let installed = ALL;
            let left: fn(Agent) -> Left = all_left;

            // Only one provider
            when(setting = setting(Some(ProviderChoice::Only(Agent::Claude)), Some(Strength::Strong))) as only_a_provider_with_usage_left {
                to take_it_on_its_strong_model { equal(Ok((Agent::Claude, Some("opus".to_string()), "only Claude".to_string()))) }
            }
            when(setting = setting(Some(ProviderChoice::Only(Agent::Claude)), None), left = claude_used_up) as only_a_provider_used_up {
                to refuse { equal(Err(used_up("Claude"))) }
            }
            when(setting = setting(Some(ProviderChoice::Only(Agent::Claude)), None), ran_out = Some(Agent::Claude)) as only_a_provider_that_just_ran_out {
                to refuse { be_err }
            }
            when(setting = setting(Some(ProviderChoice::Only(Agent::Claude)), None), author = Some(Agent::Claude)) as only_the_author_for_a_review {
                to refuse { equal(Err(only_author("Claude"))) }
            }
            when(setting = setting(Some(ProviderChoice::Only(Agent::Grok)), None), installed = &[Agent::Claude]) as only_a_provider_not_installed {
                to refuse { equal(Err(only_missing("Grok"))) }
            }
            when(setting = setting(Some(ProviderChoice::Only(Agent::Claude)), None), earlier = Some(Agent::Grok)) as only_a_provider_after_another_was_picked {
                to take_the_only_one { have(clone().map(agent_of)) equal(Ok(Agent::Claude)) }
            }

            // A preferred provider
            when(setting = setting(Some(ProviderChoice::Prefer(Agent::Claude)), Some(Strength::Fast))) as a_preferred_provider_with_usage_left {
                to take_it { equal(Ok((Agent::Claude, Some("haiku".to_string()), "preferred Claude".to_string()))) }
            }
            when(setting = setting(Some(ProviderChoice::Prefer(Agent::Claude)), Some(Strength::Strong)), left = claude_used_up) as a_preferred_provider_used_up {
                to fall_back_at_random { equal(Ok((Agent::Gemini, None, "Claude is used up, so Gemini picked at random instead of preferred Claude; Gemini has no strong model set, so it runs its default".to_string()))) }
            }
            when(setting = setting(Some(ProviderChoice::Prefer(Agent::Claude)), Some(Strength::Strong)), left = claude_used_up, earlier = Some(Agent::Codex)) as a_preferred_provider_used_up_after_an_earlier_fallback {
                to keep_the_earlier_fallback { equal(Ok((Agent::Codex, Some("gpt-6-astra".to_string()), "Claude is used up, so Codex as before instead of preferred Claude".to_string()))) }
            }
            when(setting = setting(Some(ProviderChoice::Prefer(Agent::Claude)), None), earlier = Some(Agent::Codex)) as a_preferred_provider_back_after_a_fallback {
                to go_back_to_it { have(clone().map(agent_of)) equal(Ok(Agent::Claude)) }
            }
            when(setting = setting(Some(ProviderChoice::Prefer(Agent::Claude)), None), author = Some(Agent::Claude)) as a_preferred_provider_that_wrote_the_change {
                to fall_back { have(clone().map(why_of)) equal(Ok("Claude wrote the change, so Gemini picked at random instead of preferred Claude; Gemini has no medium model set, so it runs its default".to_string())) }
            }
            when(setting = setting(Some(ProviderChoice::Prefer(Agent::Claude)), None), installed = &[Agent::Codex, Agent::Grok]) as a_preferred_provider_not_installed {
                to fall_back { have(clone().map(agent_of)) equal(Ok(Agent::Grok)) }
            }
            when(setting = setting(Some(ProviderChoice::Prefer(Agent::Claude)), None), left = none_left) as a_preferred_provider_with_nobody_to_fall_back_on {
                to refuse { equal(Err(none_left_because(" (Claude is used up; Codex is used up; Grok is used up; Gemini is used up)"))) }
            }
            when(setting = among(ProviderChoice::Prefer(Agent::Claude), &[Agent::Claude, Agent::Codex]), left = claude_used_up) as a_preferred_provider_with_fallbacks_listed {
                to fall_back_only_to_those { have(clone().map(agent_of)) equal(Ok(Agent::Codex)) }
            }

            // Any provider
            when(setting = setting(Some(ProviderChoice::Any), None)) as any_provider {
                to pick_at_random { equal(Ok((Agent::Gemini, None, "Gemini picked at random; Gemini has no medium model set, so it runs its default".to_string()))) }
            }
            when(setting = setting(None, Some(Strength::Strong))) as a_strength_without_a_provider {
                to pick_any_provider { have(clone().map(agent_of)) equal(Ok(Agent::Gemini)) }
            }
            when(setting = setting(Some(ProviderChoice::Any), Some(Strength::Strong)), earlier = Some(Agent::Claude)) as any_provider_after_an_earlier_pick {
                to keep_it { equal(Ok((Agent::Claude, Some("opus".to_string()), "Claude as before".to_string()))) }
            }
            when(setting = setting(Some(ProviderChoice::Any), None), earlier = Some(Agent::Claude), left = claude_used_up) as any_provider_after_the_earlier_pick_ran_out {
                to pick_again { have(clone().map(agent_of)) equal(Ok(Agent::Gemini)) }
            }
            when(setting = setting(Some(ProviderChoice::Any), None), earlier = Some(Agent::Claude), ran_out = Some(Agent::Claude)) as any_provider_after_the_earlier_pick_just_ran_out {
                to pick_again { have(clone().map(agent_of)) equal(Ok(Agent::Gemini)) }
            }
            when(setting = setting(Some(ProviderChoice::Any), None), author = Some(Agent::Gemini), earlier = Some(Agent::Gemini)) as any_reviewer_when_the_earlier_one_wrote_the_change {
                to leave_out_the_author { have(clone().map(agent_of)) equal(Ok(Agent::Grok)) }
            }
            when(setting = setting(Some(ProviderChoice::Any), None), left = only_grok_left) as any_provider_with_one_left {
                to take_that_one { have(clone().map(agent_of)) equal(Ok(Agent::Grok)) }
            }
            when(setting = setting(Some(ProviderChoice::Any), None), left = none_left) as any_provider_with_none_left {
                to refuse { be_err }
            }
            when(setting = among(ProviderChoice::Any, &[Agent::Claude, Agent::Codex])) as any_provider_among_some {
                to pick_among_those { have(clone().map(agent_of)) equal(Ok(Agent::Codex)) }
            }
            when(setting = among(ProviderChoice::Any, &[Agent::Grok]), installed = &[Agent::Claude]) as any_provider_among_ones_not_installed {
                to refuse { equal(Err(none_left_because(" (Grok isn't installed)"))) }
            }
        }
    }
}
