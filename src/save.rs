//! Load, edit, and write PC Legendary Edition saves.
//!
//! Mass Effect 3 Legendary uses the same PC save layout as the last original
//! Mass Effect 3 patch, so those files open here too. Xbox, PS4, and the
//! original Mass Effect 1 and 2 formats are rejected.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use crc::{Crc, CRC_32_BZIP2};
use indexmap::IndexMap;
use serde::Deserialize;

use crate::save_data::mass_effect_1::Me1MagicNumber;
use crate::save_data::mass_effect_1_le::player::Me1LeClass;
use crate::save_data::mass_effect_1_le::{Me1LeMagicNumber, Me1LeSaveGame, Me1LeVersion};
use crate::save_data::mass_effect_2::{
    Difficulty as Me2Difficulty, Me2LeSaveGame, Me2LeVersion, Me2Version,
};
use crate::save_data::mass_effect_3::{Difficulty as Me3Difficulty, Me3SaveGame, Me3Version};
use crate::save_data::shared::player::{Notoriety, Origin};
use crate::save_data::shared::plot::BitVec;
use crate::save_data::RcCell;
use crate::unreal;

const ORIGINS: &[&str] = &["None", "Spacer", "Colonist", "Earthborn"];
const NOTORIETIES: &[&str] = &["None", "Survivor", "War Hero", "Ruthless"];
const ME1_DIFFICULTY: &[&str] = &["Casual", "Normal", "Veteran", "Hardcore", "Insanity"];
const ME3_DIFFICULTY: &[&str] = &["Narrative", "Casual", "Normal", "Hardcore", "Insanity"];

const ME2_CLASSES: &[(&str, &str)] = &[
    ("SFXGame.SFXPawn_PlayerSoldier", "Soldier"),
    ("SFXGame.SFXPawn_PlayerEngineer", "Engineer"),
    ("SFXGame.SFXPawn_PlayerAdept", "Adept"),
    ("SFXGame.SFXPawn_PlayerInfiltrator", "Infiltrator"),
    ("SFXGame.SFXPawn_PlayerSentinel", "Sentinel"),
    ("SFXGame.SFXPawn_PlayerVanguard", "Vanguard"),
];

macro_rules! read_player {
    ($game:expr, |$player:ident| $body:expr) => {{
        match $game {
            Game::Me1(save) => {
                let data = save.save_data.borrow();
                let $player = data.player();
                let value = $body;
                value
            }
            Game::Me2(save) => {
                let $player = save.player();
                let value = $body;
                value
            }
            Game::Me3(save) => {
                let $player = save.player();
                let value = $body;
                value
            }
        }
    }};
}

macro_rules! write_player {
    ($game:expr, |$player:ident| $body:expr) => {{
        match $game {
            Game::Me1(save) => {
                let mut data = save.save_data.borrow_mut();
                let mut $player = data.player_mut();
                let value = $body;
                value
            }
            Game::Me2(save) => {
                let mut $player = save.player_mut();
                let value = $body;
                value
            }
            Game::Me3(save) => {
                let mut $player = save.player_mut();
                let value = $body;
                value
            }
        }
    }};
}

macro_rules! me23_read {
    ($game:expr, $method:ident) => {{
        match $game {
            Game::Me2(save) => Some(save.player().$method()),
            Game::Me3(save) => Some(save.player().$method()),
            Game::Me1(_) => None,
        }
    }};
}

macro_rules! me23_write {
    ($game:expr, $method:ident($value:expr)) => {{
        match $game {
            Game::Me2(save) => save.player_mut().$method($value),
            Game::Me3(save) => save.player_mut().$method($value),
            Game::Me1(_) => bail!("This save has no {} value.", stringify!($method)),
        }
    }};
}

const ME3_CLASSES: &[(&str, &str)] = &[
    ("SFXGame.SFXPawn_PlayerSoldier", "Soldier"),
    ("SFXGame.SFXPawn_PlayerSoldierNonCombat", "Soldier (non-combat)"),
    ("SFXGame.SFXPawn_PlayerEngineer", "Engineer"),
    ("SFXGame.SFXPawn_PlayerEngineerNonCombat", "Engineer (non-combat)"),
    ("SFXGame.SFXPawn_PlayerAdept", "Adept"),
    ("SFXGame.SFXPawn_PlayerAdeptNonCombat", "Adept (non-combat)"),
    ("SFXGame.SFXPawn_PlayerInfiltrator", "Infiltrator"),
    ("SFXGame.SFXPawn_PlayerInfiltratorNonCombat", "Infiltrator (non-combat)"),
    ("SFXGame.SFXPawn_PlayerSentinel", "Sentinel"),
    ("SFXGame.SFXPawn_PlayerSentinelNonCombat", "Sentinel (non-combat)"),
    ("SFXGame.SFXPawn_PlayerVanguard", "Vanguard"),
    ("SFXGame.SFXPawn_PlayerVanguardNonCombat", "Vanguard (non-combat)"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Name,
    Gender,
    Origin,
    Notoriety,
    Class,
    Level,
    Xp,
    TalentPoints,
    Credits,
    Medigel,
    Grenades,
    Omnigel,
    Eezo,
    Iridium,
    Palladium,
    Platinum,
    Probes,
    Fuel,
    Paragon,
    Renegade,
    Reputation,
    ReputationPoints,
    Difficulty,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Gender => "Gender",
            Field::Origin => "Origin",
            Field::Notoriety => "Notoriety",
            Field::Class => "Class",
            Field::Level => "Level",
            Field::Xp => "Current XP",
            Field::TalentPoints => "Talent Points",
            Field::Credits => "Credits",
            Field::Medigel => "Medigel",
            Field::Grenades => "Grenades",
            Field::Omnigel => "Omnigel",
            Field::Eezo => "Eezo",
            Field::Iridium => "Iridium",
            Field::Palladium => "Palladium",
            Field::Platinum => "Platinum",
            Field::Probes => "Probes",
            Field::Fuel => "Fuel",
            Field::Paragon => "Paragon",
            Field::Renegade => "Renegade",
            Field::Reputation => "Reputation",
            Field::ReputationPoints => "Reputation Points",
            Field::Difficulty => "Difficulty",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Text,
    Int,
    Float,
    Choice,
    Readonly,
}

pub struct Row {
    pub group: &'static str,
    pub field: Field,
}

pub struct SaveReport {
    /// Set when this save created `file.pcsav.bak` from the untouched original.
    pub backup: Option<PathBuf>,
}

pub enum Game {
    Me1(Me1LeSaveGame),
    Me2(Me2LeSaveGame),
    Me3(Me3SaveGame),
}

impl Game {
    pub fn load_path(path: &Path) -> Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;
        Self::load(&bytes).with_context(|| format!("Could not open {}", path.display()))
    }

    pub fn load(bytes: &[u8]) -> Result<Self> {
        if peek::<Me1MagicNumber>(bytes) {
            bail!("This editor opens PC Legendary Edition saves. This file is an original Mass Effect 1 save.");
        }
        if peek::<Me1LeMagicNumber>(bytes) {
            let save = unreal::Deserializer::from_bytes(bytes).context(
                "This looks like a Mass Effect 1 Legendary PC save, but it could not be read",
            )?;
            return Ok(Game::Me1(save));
        }
        if peek::<Me1LeVersion>(bytes) {
            bail!("This editor opens PC Legendary Edition saves. This file is a PS4 or uncompressed Mass Effect 1 save.");
        }
        if let Ok(version) = unreal::Deserializer::from_bytes::<Me2Version>(bytes) {
            if version.is_xbox360 {
                bail!("This editor opens PC Legendary Edition saves. This file is an Xbox 360 Mass Effect 2 save.");
            }
            bail!("This editor opens PC Legendary Edition saves. This file is an original Mass Effect 2 save.");
        }
        if peek::<Me2LeVersion>(bytes) {
            let save = unreal::Deserializer::from_bytes(bytes).context(
                "This looks like a Mass Effect 2 Legendary save, but it could not be read",
            )?;
            return Ok(Game::Me2(save));
        }
        if let Ok(version) = unreal::Deserializer::from_bytes::<Me3Version>(bytes) {
            if version.is_xbox360 {
                bail!("This editor opens PC Legendary Edition saves. This file is an Xbox 360 Mass Effect 3 save.");
            }
            let save = unreal::Deserializer::from_bytes(bytes)
                .context("This looks like a Mass Effect 3 PC save, but it could not be read")?;
            return Ok(Game::Me3(save));
        }
        bail!("This is not a PC Legendary Edition save (.pcsav).");
    }

    pub fn save_to(&self, path: &Path) -> Result<SaveReport> {
        let bytes = self.to_bytes()?;
        let backup = write_backup_once(path)?;
        let tmp = sibling(path, ".tmp");
        fs::write(&tmp, &bytes).with_context(|| format!("Could not write {}", tmp.display()))?;
        if let Err(err) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(err).with_context(|| format!("Could not replace {}", path.display()));
        }
        Ok(SaveReport { backup })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        match self {
            Game::Me1(save) => checksum_me1(save),
            Game::Me2(save) => checksum_appended(save),
            Game::Me3(save) => checksum_appended(save),
        }
    }

    pub fn title(&self) -> &'static str {
        match self {
            Game::Me1(_) => "Mass Effect 1 Legendary",
            Game::Me2(_) => "Mass Effect 2 Legendary",
            Game::Me3(_) => "Mass Effect 3",
        }
    }

    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut push = |group, field| rows.push(Row { group, field });

        push("Character", Field::Name);
        push("Character", Field::Gender);
        push("Character", Field::Origin);
        push("Character", Field::Notoriety);
        push("Character", Field::Class);
        push("Character", Field::Level);
        push("Character", Field::Xp);
        push("Character", Field::TalentPoints);

        push("Resources", Field::Credits);
        push("Resources", Field::Medigel);
        match self {
            Game::Me1(_) => {
                push("Resources", Field::Grenades);
                push("Resources", Field::Omnigel);
            }
            Game::Me2(_) => {
                push("Resources", Field::Eezo);
                push("Resources", Field::Iridium);
                push("Resources", Field::Palladium);
                push("Resources", Field::Platinum);
                push("Resources", Field::Probes);
                push("Resources", Field::Fuel);
            }
            Game::Me3(_) => {
                push("Resources", Field::Grenades);
                push("Resources", Field::Eezo);
                push("Resources", Field::Iridium);
                push("Resources", Field::Palladium);
                push("Resources", Field::Platinum);
                push("Resources", Field::Probes);
                push("Resources", Field::Fuel);
            }
        }

        push("Morality", Field::Paragon);
        push("Morality", Field::Renegade);
        if matches!(self, Game::Me3(_)) {
            push("Morality", Field::Reputation);
            push("Morality", Field::ReputationPoints);
        }

        push("General", Field::Difficulty);
        rows
    }

    pub fn input_kind(&self, field: Field) -> InputKind {
        match field {
            Field::Name => InputKind::Text,
            Field::Gender | Field::Origin | Field::Notoriety | Field::Difficulty => {
                InputKind::Choice
            }
            Field::Class => match self {
                Game::Me1(_) => InputKind::Readonly,
                Game::Me2(_) | Game::Me3(_) => InputKind::Choice,
            },
            Field::Level
            | Field::TalentPoints
            | Field::Credits
            | Field::Medigel
            | Field::Eezo
            | Field::Iridium
            | Field::Palladium
            | Field::Platinum
            | Field::Probes
            | Field::Paragon
            | Field::Renegade
            | Field::Reputation
            | Field::ReputationPoints => InputKind::Int,
            Field::Xp | Field::Fuel | Field::Omnigel => InputKind::Float,
            Field::Grenades => match self {
                Game::Me1(_) => InputKind::Float,
                Game::Me3(_) => InputKind::Int,
                Game::Me2(_) => InputKind::Readonly,
            },
        }
    }

    /// `None` when this save does not store that value.
    pub fn value(&self, field: Field) -> Option<String> {
        let text = match field {
            Field::Name => read_player!(self, |player| player.first_name().to_string()),
            Field::Gender => {
                if read_player!(self, |player| player.is_female()) {
                    "Female".to_string()
                } else {
                    "Male".to_string()
                }
            }
            Field::Origin => {
                ORIGINS[origin_index(read_player!(self, |player| *player.origin()))].to_string()
            }
            Field::Notoriety => NOTORIETIES
                [notoriety_index(read_player!(self, |player| *player.notoriety()))]
            .to_string(),
            Field::Class => self.class_value(),
            Field::Level => read_player!(self, |player| player.level()).to_string(),
            Field::Xp => format_f32(read_player!(self, |player| player.current_xp())),
            Field::TalentPoints => read_player!(self, |player| player.talent_points()).to_string(),
            Field::Credits => read_player!(self, |player| player.credits()).to_string(),
            Field::Medigel => read_player!(self, |player| player.medigel()).to_string(),
            Field::Grenades => match self {
                Game::Me1(save) => {
                    let data = save.save_data.borrow();
                    let player = data.player();
                    format_f32(player.grenades())
                }
                Game::Me3(save) => save.player().grenades().to_string(),
                Game::Me2(_) => return None,
            },
            Field::Omnigel => match self {
                Game::Me1(save) => {
                    let data = save.save_data.borrow();
                    let player = data.player();
                    format_f32(player.omnigel())
                }
                Game::Me2(_) | Game::Me3(_) => return None,
            },
            Field::Eezo => return me23_read!(self, eezo).map(|value| value.to_string()),
            Field::Iridium => return me23_read!(self, iridium).map(|value| value.to_string()),
            Field::Palladium => return me23_read!(self, palladium).map(|value| value.to_string()),
            Field::Platinum => return me23_read!(self, platinum).map(|value| value.to_string()),
            Field::Probes => return me23_read!(self, probes).map(|value| value.to_string()),
            Field::Fuel => return me23_read!(self, current_fuel).map(format_f32),
            Field::Paragon => return self.morality(47, 2, 10159).map(|value| value.to_string()),
            Field::Renegade => return self.morality(46, 3, 10160).map(|value| value.to_string()),
            Field::Reputation => return self.me3_int(10297).map(|value| value.to_string()),
            Field::ReputationPoints => return self.me3_int(10380).map(|value| value.to_string()),
            Field::Difficulty => return self.difficulty_value(),
        };
        Some(text)
    }

    pub fn cycle(&mut self, field: Field, delta: i32) -> Result<()> {
        if matches!((&*self, field), (Game::Me1(_), Field::Class)) {
            bail!("Changing class isn't supported yet. It resets talents and unequips gear.");
        }
        if self.input_kind(field) != InputKind::Choice {
            bail!("Press enter to type a new value.");
        }
        match field {
            Field::Gender => {
                let female = !read_player!(self, |player| player.is_female());
                self.set_gender(female);
            }
            Field::Origin => {
                let next = cycle_index(
                    origin_index(read_player!(self, |player| *player.origin())) as i32,
                    ORIGINS.len(),
                    delta,
                );
                self.set_origin(next);
            }
            Field::Notoriety => {
                let next = cycle_index(
                    notoriety_index(read_player!(self, |player| *player.notoriety())) as i32,
                    NOTORIETIES.len(),
                    delta,
                );
                self.set_notoriety(next);
            }
            Field::Class => self.cycle_class(delta)?,
            Field::Difficulty => self.cycle_difficulty(delta)?,
            _ => bail!("Press enter to type a new value."),
        }
        Ok(())
    }

    pub fn apply_text(&mut self, field: Field, text: &str) -> Result<()> {
        match self.input_kind(field) {
            InputKind::Text => self.set_name(text.trim()),
            InputKind::Int => {
                let value = text
                    .trim()
                    .parse::<i32>()
                    .map_err(|_| anyhow::anyhow!("Enter a whole number."))?;
                self.set_int(field, value)?;
            }
            InputKind::Float => {
                let value =
                    text.trim().parse::<f32>().map_err(|_| anyhow::anyhow!("Enter a number."))?;
                if !value.is_finite() {
                    bail!("Enter a finite number.");
                }
                self.set_float(field, value)?;
            }
            InputKind::Choice => bail!("Use left and right to change this."),
            InputKind::Readonly => {
                if matches!((&*self, field), (Game::Me1(_), Field::Class)) {
                    bail!(
                        "Changing class isn't supported yet. It resets talents and unequips gear."
                    );
                }
                bail!("This value can't be edited.");
            }
        }
        Ok(())
    }

    fn set_name(&mut self, name: &str) {
        write_player!(self, |player| *player.first_name_mut() = name.to_string());
    }

    fn set_int(&mut self, field: Field, value: i32) -> Result<()> {
        match field {
            Field::Level => write_player!(self, |player| player.set_level(value)),
            Field::TalentPoints => write_player!(self, |player| player.set_talent_points(value)),
            Field::Credits => write_player!(self, |player| player.set_credits(value)),
            Field::Medigel => write_player!(self, |player| player.set_medigel(value)),
            Field::Grenades => match self {
                Game::Me3(save) => save.player_mut().set_grenades(value),
                Game::Me1(_) | Game::Me2(_) => {
                    bail!("Grenades in this save are not a whole number.")
                }
            },
            Field::Eezo => me23_write!(self, set_eezo(value)),
            Field::Iridium => me23_write!(self, set_iridium(value)),
            Field::Palladium => me23_write!(self, set_palladium(value)),
            Field::Platinum => me23_write!(self, set_platinum(value)),
            Field::Probes => me23_write!(self, set_probes(value)),
            Field::Paragon => self.set_morality(47, 2, 10159, value, "Paragon")?,
            Field::Renegade => self.set_morality(46, 3, 10160, value, "Renegade")?,
            Field::Reputation => self.set_me3_int(10297, value, "Reputation")?,
            Field::ReputationPoints => self.set_me3_int(10380, value, "Reputation Points")?,
            _ => bail!("This value can't be edited as a whole number."),
        }
        Ok(())
    }

    fn set_float(&mut self, field: Field, value: f32) -> Result<()> {
        match field {
            Field::Xp => write_player!(self, |player| player.set_current_xp(value)),
            Field::Fuel => me23_write!(self, set_current_fuel(value)),
            Field::Grenades => match self {
                Game::Me1(save) => {
                    let mut data = save.save_data.borrow_mut();
                    data.player_mut().set_grenades(value);
                }
                Game::Me2(_) | Game::Me3(_) => bail!("This save has no grenade count to edit."),
            },
            Field::Omnigel => match self {
                Game::Me1(save) => {
                    let mut data = save.save_data.borrow_mut();
                    data.player_mut().set_omnigel(value);
                }
                Game::Me2(_) | Game::Me3(_) => bail!("This save has no omnigel count."),
            },
            _ => bail!("This value can't be edited as a decimal."),
        }
        Ok(())
    }

    fn set_gender(&mut self, female: bool) {
        write_player!(self, |player| player.set_is_female(female));
        match self {
            Game::Me1(save) => {
                let mut data = save.save_data.borrow_mut();
                let mut plot = data.plot_mut();
                set_bit(&mut plot.booleans_mut(), 4639, female);
            }
            Game::Me2(save) => {
                {
                    let mut plot = save.me1_plot_mut();
                    set_bit(&mut plot.booleans_mut(), 4639, female);
                }
                {
                    let mut plot = save.plot_mut();
                    set_bit(&mut plot.booleans_mut(), 66, female);
                }
            }
            Game::Me3(save) => {
                let mut plot = save.plot_mut();
                set_bit(&mut plot.booleans_mut(), 14639, female);
                set_bit(&mut plot.booleans_mut(), 66, female);
                set_bit(&mut plot.booleans_mut(), 17662, female);
                let loco = get_bit(&plot.booleans(), 19578).unwrap_or(false);
                let lola = get_bit(&plot.booleans(), 19579).unwrap_or(false);
                if loco || lola {
                    set_bit(&mut plot.booleans_mut(), 19578, !female);
                    set_bit(&mut plot.booleans_mut(), 19579, female);
                }
            }
        }
    }

    fn set_origin(&mut self, index: usize) {
        let origin = origin_from(index);
        write_player!(self, |player| *player.origin_mut() = origin);
        match self {
            Game::Me1(save) => {
                let mut data = save.save_data.borrow_mut();
                let mut plot = data.plot_mut();
                set_vec_int(&mut plot.integers_mut(), 1, index as i32);
            }
            Game::Me2(save) => {
                {
                    let mut plot = save.plot_mut();
                    set_origin_flags(&mut plot.booleans_mut(), index);
                }
                {
                    let mut plot = save.me1_plot_mut();
                    set_vec_int(&mut plot.integers_mut(), 1, index as i32);
                }
            }
            Game::Me3(save) => {
                let mut plot = save.plot_mut();
                set_origin_flags(&mut plot.booleans_mut(), index);
                set_map_int(&mut plot.integers_mut(), 10001, index as i32);
            }
        }
    }

    fn set_notoriety(&mut self, index: usize) {
        let notoriety = notoriety_from(index);
        write_player!(self, |player| *player.notoriety_mut() = notoriety);
        match self {
            Game::Me1(save) => {
                let mut data = save.save_data.borrow_mut();
                let mut plot = data.plot_mut();
                set_vec_int(&mut plot.integers_mut(), 2, index as i32);
            }
            Game::Me2(save) => {
                {
                    let mut plot = save.plot_mut();
                    set_notoriety_flags(&mut plot.booleans_mut(), index);
                }
                {
                    let mut plot = save.me1_plot_mut();
                    set_vec_int(&mut plot.integers_mut(), 2, index as i32);
                }
            }
            Game::Me3(save) => {
                let mut plot = save.plot_mut();
                set_notoriety_flags(&mut plot.booleans_mut(), index);
                set_map_int(&mut plot.integers_mut(), 10002, index as i32);
            }
        }
    }

    fn cycle_class(&mut self, delta: i32) -> Result<()> {
        let classes: &[(&str, &str)] = match self {
            Game::Me1(_) => {
                bail!("Changing class isn't supported yet. It resets talents and unequips gear.")
            }
            Game::Me2(_) => ME2_CLASSES,
            Game::Me3(_) => ME3_CLASSES,
        };
        let current = self.raw_class();
        let next = match class_index(&current, classes) {
            Some(index) => cycle_index(index as i32, classes.len(), delta),
            None if delta < 0 => classes.len() - 1,
            None => 0,
        };
        let internal = classes[next].0;
        match self {
            Game::Me2(save) => {
                let mut player = save.player_mut();
                *player.class_name_mut() = internal.to_string();
            }
            Game::Me3(save) => {
                let mut player = save.player_mut();
                *player.class_name_mut() = internal.to_string();
            }
            Game::Me1(_) => unreachable!(),
        }
        Ok(())
    }

    fn cycle_difficulty(&mut self, delta: i32) -> Result<()> {
        match self {
            Game::Me1(save) => {
                let current =
                    me1_difficulty(save).context("This save has no difficulty setting.")?;
                let next = cycle_index(current, ME1_DIFFICULTY.len(), delta);
                let mut data = save.save_data.borrow_mut();
                let mut player = data.player_mut();
                let mut options = player.game_options_mut();
                let Some(cell) = options.first_mut() else {
                    bail!("This save has no difficulty setting.");
                };
                cell.set(next as i32);
            }
            Game::Me2(save) => {
                let current = me2_difficulty_index(&save.difficulty()) as i32;
                let next = cycle_index(current, ME1_DIFFICULTY.len(), delta);
                *save.difficulty_mut() = me2_difficulty_from(next);
            }
            Game::Me3(save) => {
                let current = me3_difficulty_index(&save.difficulty()) as i32;
                let next = cycle_index(current, ME3_DIFFICULTY.len(), delta);
                *save.difficulty_mut() = me3_difficulty_from(next);
            }
        }
        Ok(())
    }

    fn class_value(&self) -> String {
        match self {
            Game::Me1(save) => me1_class_label(save).to_string(),
            Game::Me2(_) => pretty_class(&self.raw_class(), ME2_CLASSES),
            Game::Me3(_) => pretty_class(&self.raw_class(), ME3_CLASSES),
        }
    }

    fn raw_class(&self) -> String {
        match self {
            Game::Me1(save) => me1_class_label(save).to_string(),
            Game::Me2(save) => save.player().class_name().to_string(),
            Game::Me3(save) => save.player().class_name().to_string(),
        }
    }

    fn difficulty_value(&self) -> Option<String> {
        match self {
            Game::Me1(save) => {
                me1_difficulty(save).map(|index| named_or_number(index, ME1_DIFFICULTY))
            }
            Game::Me2(save) => {
                Some(ME1_DIFFICULTY[me2_difficulty_index(&save.difficulty())].to_string())
            }
            Game::Me3(save) => {
                Some(ME3_DIFFICULTY[me3_difficulty_index(&save.difficulty())].to_string())
            }
        }
    }

    /// Plot integers: ME1 vec index, ME2 vec index, ME3 map id.
    fn morality(&self, me1: usize, me2: usize, me3: i32) -> Option<i32> {
        match self {
            Game::Me1(save) => {
                let data = save.save_data.borrow();
                let plot = data.plot();
                let integers = plot.integers();
                get_vec_int(&integers, me1)
            }
            Game::Me2(save) => {
                let plot = save.plot();
                let integers = plot.integers();
                get_vec_int(&integers, me2)
            }
            Game::Me3(save) => {
                let plot = save.plot();
                let integers = plot.integers();
                get_map_int(&integers, me3)
            }
        }
    }

    fn set_morality(
        &mut self, me1: usize, me2: usize, me3: i32, value: i32, label: &str,
    ) -> Result<()> {
        let updated = match self {
            Game::Me1(save) => {
                let mut data = save.save_data.borrow_mut();
                let mut plot = data.plot_mut();
                let mut integers = plot.integers_mut();
                set_vec_int(&mut integers, me1, value)
            }
            Game::Me2(save) => {
                let mut plot = save.plot_mut();
                let mut integers = plot.integers_mut();
                set_vec_int(&mut integers, me2, value)
            }
            Game::Me3(save) => {
                let mut plot = save.plot_mut();
                let mut integers = plot.integers_mut();
                set_map_int(&mut integers, me3, value)
            }
        };
        if updated {
            Ok(())
        } else {
            bail!("This save has no {label} value.");
        }
    }

    fn me3_int(&self, id: i32) -> Option<i32> {
        match self {
            Game::Me3(save) => {
                let plot = save.plot();
                let integers = plot.integers();
                get_map_int(&integers, id)
            }
            Game::Me1(_) | Game::Me2(_) => None,
        }
    }

    fn set_me3_int(&mut self, id: i32, value: i32, label: &str) -> Result<()> {
        let Game::Me3(save) = self else {
            bail!("This save has no {label} value.");
        };
        let mut plot = save.plot_mut();
        let mut integers = plot.integers_mut();
        if set_map_int(&mut integers, id, value) {
            Ok(())
        } else {
            bail!("This save has no {label} value.");
        }
    }

    #[cfg(test)]
    fn is_female(&self) -> bool {
        read_player!(self, |player| player.is_female())
    }

    #[cfg(test)]
    fn main_gender_flag(&self) -> Option<bool> {
        match self {
            Game::Me1(save) => {
                let data = save.save_data.borrow();
                let plot = data.plot();
                let bits = plot.booleans();
                get_bit(&bits, 4639)
            }
            Game::Me2(save) => {
                let plot = save.plot();
                let bits = plot.booleans();
                get_bit(&bits, 66)
            }
            Game::Me3(save) => {
                let plot = save.plot();
                let bits = plot.booleans();
                get_bit(&bits, 17662)
            }
        }
    }

    #[cfg(test)]
    fn stats(&self) -> Stats {
        Stats {
            name: self.value(Field::Name).unwrap_or_default(),
            female: self.is_female(),
            level: read_player!(self, |player| player.level()),
            xp_bits: read_player!(self, |player| player.current_xp()).to_bits(),
            credits: read_player!(self, |player| player.credits()),
            medigel: read_player!(self, |player| player.medigel()),
        }
    }
}

#[cfg(test)]
#[derive(Debug, PartialEq)]
struct Stats {
    name: String,
    female: bool,
    level: i32,
    xp_bits: u32,
    credits: i32,
    medigel: i32,
}

fn peek<T>(bytes: &[u8]) -> bool
where
    T: for<'de> Deserialize<'de>,
{
    unreal::Deserializer::from_bytes::<T>(bytes).is_ok()
}

fn checksum_appended(save: &impl serde::Serialize) -> Result<Vec<u8>> {
    let mut output = unreal::Serializer::to_vec(save)?;
    let checksum = crc32(&output);
    output.extend(u32::to_le_bytes(checksum));
    Ok(output)
}

fn checksum_me1(save: &Me1LeSaveGame) -> Result<Vec<u8>> {
    let mut output = unreal::Serializer::to_vec(save)?;
    if output.len() < 12 {
        bail!("Save is too small to checksum.");
    }
    let checksum_offset = output.len() - 12;
    let checksum = crc32(&output[..checksum_offset]);
    output[checksum_offset..checksum_offset + 4].copy_from_slice(&u32::to_le_bytes(checksum));
    Ok(output)
}

fn crc32(bytes: &[u8]) -> u32 {
    Crc::<u32>::new(&CRC_32_BZIP2).checksum(bytes)
}

fn write_backup_once(path: &Path) -> Result<Option<PathBuf>> {
    let bak = sibling(path, ".bak");
    if bak.exists() {
        return Ok(None);
    }
    fs::copy(path, &bak)
        .with_context(|| format!("Could not back up the save to {}", bak.display()))?;
    Ok(Some(bak))
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

fn format_f32(value: f32) -> String {
    if value.is_finite() && value.fract() == 0.0 && value.abs() < 1.0e9 {
        format!("{}", value as i64)
    } else {
        let text = format!("{value:.4}");
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

fn cycle_index(current: i32, len: usize, delta: i32) -> usize {
    let len = len as i32;
    if (0..len).contains(&current) {
        (current + delta).rem_euclid(len) as usize
    } else if delta < 0 {
        (len - 1) as usize
    } else {
        0
    }
}

fn named_or_number(index: i32, names: &[&str]) -> String {
    usize::try_from(index)
        .ok()
        .and_then(|index| names.get(index).copied())
        .map(str::to_string)
        .unwrap_or_else(|| index.to_string())
}

fn origin_index(origin: Origin) -> usize {
    match origin {
        Origin::None => 0,
        Origin::Spacer => 1,
        Origin::Colonist => 2,
        Origin::Earthborn => 3,
    }
}

fn origin_from(index: usize) -> Origin {
    match index {
        1 => Origin::Spacer,
        2 => Origin::Colonist,
        3 => Origin::Earthborn,
        _ => Origin::None,
    }
}

fn notoriety_index(notoriety: Notoriety) -> usize {
    match notoriety {
        Notoriety::None => 0,
        Notoriety::Survivor => 1,
        Notoriety::Warhero => 2,
        Notoriety::Ruthless => 3,
    }
}

fn notoriety_from(index: usize) -> Notoriety {
    match index {
        1 => Notoriety::Survivor,
        2 => Notoriety::Warhero,
        3 => Notoriety::Ruthless,
        _ => Notoriety::None,
    }
}

fn me1_class_label(save: &Me1LeSaveGame) -> &'static str {
    let data = save.save_data.borrow();
    let player = data.player();
    let class = Me1LeClass::clone(&player.player_class());
    match class {
        Me1LeClass::Soldier => "Soldier",
        Me1LeClass::Engineer => "Engineer",
        Me1LeClass::Adept => "Adept",
        Me1LeClass::Infiltrator => "Infiltrator",
        Me1LeClass::Sentinel => "Sentinel",
        Me1LeClass::Vanguard => "Vanguard",
    }
}

fn me1_difficulty(save: &Me1LeSaveGame) -> Option<i32> {
    let data = save.save_data.borrow();
    let player = data.player();
    let options = player.game_options();
    options.first().map(|cell| cell.get())
}

fn me2_difficulty_index(difficulty: &Me2Difficulty) -> usize {
    match difficulty {
        Me2Difficulty::Casual => 0,
        Me2Difficulty::Normal => 1,
        Me2Difficulty::Veteran => 2,
        Me2Difficulty::Hardcore => 3,
        Me2Difficulty::Insanity => 4,
    }
}

fn me2_difficulty_from(index: usize) -> Me2Difficulty {
    match index {
        1 => Me2Difficulty::Normal,
        2 => Me2Difficulty::Veteran,
        3 => Me2Difficulty::Hardcore,
        4 => Me2Difficulty::Insanity,
        _ => Me2Difficulty::Casual,
    }
}

fn me3_difficulty_index(difficulty: &Me3Difficulty) -> usize {
    match difficulty {
        Me3Difficulty::Narrative => 0,
        Me3Difficulty::Casual => 1,
        Me3Difficulty::Normal => 2,
        Me3Difficulty::Hardcore => 3,
        Me3Difficulty::Insanity => 4,
    }
}

fn me3_difficulty_from(index: usize) -> Me3Difficulty {
    match index {
        1 => Me3Difficulty::Casual,
        2 => Me3Difficulty::Normal,
        3 => Me3Difficulty::Hardcore,
        4 => Me3Difficulty::Insanity,
        _ => Me3Difficulty::Narrative,
    }
}

fn class_index(current: &str, classes: &[(&str, &str)]) -> Option<usize> {
    classes.iter().position(|(internal, _)| current.eq_ignore_ascii_case(internal))
}

fn pretty_class(current: &str, classes: &[(&str, &str)]) -> String {
    class_index(current, classes)
        .map(|index| classes[index].1.to_string())
        .unwrap_or_else(|| current.to_string())
}

fn set_bit(bits: &mut BitVec, index: usize, value: bool) {
    if let Some(mut bit) = bits.get_mut(index) {
        *bit = value;
    }
}

fn get_bit(bits: &BitVec, index: usize) -> Option<bool> {
    bits.get(index).map(|bit| *bit)
}

fn get_vec_int(values: &[RcCell<i32>], index: usize) -> Option<i32> {
    values.get(index).map(|cell| cell.get())
}

fn set_vec_int(values: &mut [RcCell<i32>], index: usize, value: i32) -> bool {
    match values.get_mut(index) {
        Some(cell) => {
            cell.set(value);
            true
        }
        None => false,
    }
}

fn get_map_int(values: &IndexMap<i32, RcCell<i32>>, id: i32) -> Option<i32> {
    values.get(&id).map(|cell| cell.get())
}

fn set_map_int(values: &mut IndexMap<i32, RcCell<i32>>, id: i32, value: i32) -> bool {
    match values.get_mut(&id) {
        Some(cell) => {
            cell.set(value);
            true
        }
        None => false,
    }
}

fn set_origin_flags(bits: &mut BitVec, index: usize) {
    if index == 0 {
        return;
    }
    set_bit(bits, 1533, index == 1);
    set_bit(bits, 1534, index == 3);
    set_bit(bits, 1535, index == 2);
}

fn set_notoriety_flags(bits: &mut BitVec, index: usize) {
    if index == 0 {
        return;
    }
    set_bit(bits, 1537, index == 1);
    set_bit(bits, 1538, index == 2);
    set_bit(bits, 1539, index == 3);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("test").join(name);
        fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
    }

    #[test]
    fn legendary_pc_roundtrip() {
        for file in ["ME1LeSave.pcsav", "ME1LeExport.pcsav", "ME2LeSave.pcsav", "ME3Save.pcsav"] {
            let game = Game::load(&fixture(file)).unwrap_or_else(|err| panic!("{file}: {err:#}"));
            let stats = game.stats();
            let encoded = game.to_bytes().unwrap_or_else(|err| panic!("{file}: {err:#}"));
            let reloaded =
                Game::load(&encoded).unwrap_or_else(|err| panic!("{file} reload: {err:#}"));
            assert_eq!(reloaded.stats(), stats, "{file}");

            let mut edited = game;
            let credits = edited.stats().credits.saturating_add(1);
            edited.apply_text(Field::Credits, &credits.to_string()).unwrap();
            edited.apply_text(Field::Name, "Tester").unwrap();
            edited.cycle(Field::Gender, 1).unwrap();
            let encoded = edited.to_bytes().unwrap();
            let reloaded = Game::load(&encoded).unwrap();
            assert_eq!(reloaded.stats().credits, credits, "{file}");
            assert_eq!(reloaded.value(Field::Name).as_deref(), Some("Tester"), "{file}");
            assert_eq!(reloaded.is_female(), !stats.female, "{file}");
            if let Some(flag) = reloaded.main_gender_flag() {
                assert_eq!(flag, reloaded.is_female(), "{file} gender plot flag");
            }
        }
    }

    #[test]
    fn rejects_other_saves() {
        for (file, hint) in [
            ("ME1Save.MassEffectSave", "original Mass Effect 1"),
            ("ME2Save.pcsav", "original Mass Effect 2"),
            ("ME2Save360.xbsav", "Xbox"),
            ("ME3Save360.xbsav", "Xbox"),
        ] {
            let Err(err) = Game::load(&fixture(file)) else {
                panic!("{file} was accepted");
            };
            let message = format!("{err:#}");
            assert!(message.contains(hint), "{file} error was: {message}");
        }
    }
}
