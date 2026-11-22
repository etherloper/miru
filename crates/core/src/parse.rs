//! Turns release-style file paths into movie or episode identities.
//!
//! Handles the common naming schemes: `Show.S01E02.Title.1080p.mkv`, `Show 1x02.mkv`,
//! `Show/Season 1/02 - Title.mkv`, `Movie.Title.2010.BluRay.mkv` and
//! `Movie Title (2010)/movie.mkv`.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// Bump when parsing rules change, so the next scan re-parses every file.
pub const PARSER_VERSION: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedMedia {
    Movie {
        title: String,
        year: Option<u16>,
    },
    Episode {
        show: String,
        season: u16,
        episode: u16,
        /// Last episode of a multi-episode file (`S01E01E02`, `S01E01-03`).
        episode_end: Option<u16>,
        title: Option<String>,
    },
}

static SXXEYY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bs(\d{1,2})[ ._-]?e(\d{1,3})(?:[ ._-]?-?e(\d{1,3})|-(\d{1,3})\b)?").unwrap()
});
static NXNN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{2,3})\b").unwrap());
static SEASON_DIR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:season|series|s)[ ._-]*(\d{1,2})$").unwrap());
/// A season folder that also names the show: `Show S01`, `Show.Season.2.1080p`, `Show S01 [1986]`.
static NAMED_SEASON_DIR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.+?)[ ._-]+(?:season|series|s)[ ._-]*(\d{1,2})(?:$|[ ._\-\[(])").unwrap()
});
/// `Show Name 01`, `Show.Name.E01.Title`: the show's name, then a two- or three-digit number.
static NUMBERED_AFTER_SHOW: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(.+?)[ ._-]+(?:e|ep|episode)?[ ._-]*(\d{2,3})(?:[ ._-]+(.*))?$").unwrap()
});
/// `01 - Title`, `01. Title`: a two- or three-digit number, a separator and the episode title.
static NUMBERED_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d{2,3})\s*(?:-|\.)\s+(.+)$").unwrap());
static LEADING_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:e|ep|episode)?[ ._-]*(\d{1,3})\b").unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:19|20)\d{2}").unwrap());
/// Release tags that mark the end of the human-readable part of a name.
static JUNK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?ix)(?:^|[\s.\[(_-])(?:
            2160p|1080[pi]|720p|576p|480p|4k|uhd|hdr(?:10\+?)?|dolby[\s.]?vision|dv|sdr|
            blu-?ray|bdrip|brrip|bdremux|remux|web-?dl|web-?rip|web|hdtv|dvdrip|dvd|hdrip|
            x264|x265|h[.\s]?26[45]|hevc|avc|av1|xvid|10-?bit|
            aac(?:2[.\s]0)?|ac3|eac3|dts(?:-?hd)?|truehd|atmos|ddp?(?:5[.\s]1|2[.\s]0)?|flac|
            proper|repack|extended|unrated|imax|remastered|internal|multi|subbed
        )(?:$|[\s.\])_-])",
    )
    .unwrap()
});
static BRACKETED_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\[(]\s*((?:19|20)\d{2})\s*[\])]").unwrap());
static BRACKETED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\{[^}]*\}").unwrap());

/// Parses a media file path. `path` may be absolute or relative to a library root; parent
/// folder names are used when the file name alone is ambiguous.
pub fn parse_path(path: &Path) -> ParsedMedia {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let parents: Vec<&str> = path
        .ancestors()
        .skip(1)
        .filter_map(|p| p.file_name()?.to_str())
        .collect();

    if let Some(mut episode) = parse_episode(stem, &parents) {
        if let ParsedMedia::Episode { show, .. } = &mut episode {
            *show = canonical_show(show);
        }
        return episode;
    }
    parse_movie(stem, &parents)
}

fn parse_episode(stem: &str, parents: &[&str]) -> Option<ParsedMedia> {
    let explicit = SXXEYY
        .captures(stem)
        .map(|c| {
            let end = c
                .get(3)
                .or_else(|| c.get(4))
                .and_then(|m| m.as_str().parse().ok());
            (
                c.get(0).map_or(0..0, |m| m.range()),
                num(&c, 1),
                num(&c, 2),
                end,
            )
        })
        .or_else(|| {
            NXNN.captures(stem).map(|c| {
                (
                    c.get(0).map_or(0..0, |m| m.range()),
                    num(&c, 1),
                    num(&c, 2),
                    None,
                )
            })
        });

    if let Some((range, season, episode, episode_end)) = explicit {
        // `Show.2026.S01E01`: release names add the show's year, which isn't part of its name.
        let before = title_and_year(&stem[..range.start]).0;
        let after = clean_title(&stem[range.end..]);
        let show = if before.is_empty() {
            show_from_parents(parents)?
        } else {
            before
        };
        return Some(ParsedMedia::Episode {
            show,
            season,
            episode,
            episode_end: episode_end.filter(|&e| e > episode),
            title: non_empty(after),
        });
    }

    // `Show/Season 2/05 - Title.mkv`: numbering lives in the folder structure.
    let Some(season) = parents.first().and_then(|dir| season_of_dir(dir)) else {
        return single_season_episode(stem, parents);
    };
    let lead = LEADING_EPISODE.captures(stem)?;
    let after = clean_title(&stem[lead.get(0).map_or(0, |m| m.end())..]);
    Some(ParsedMedia::Episode {
        show: show_from_parents(parents)?,
        season,
        episode: num(&lead, 1),
        episode_end: None,
        title: non_empty(after),
    })
}

/// Shows with one season and no season folders, numbered inside the show folder:
/// `Our Blues [2022]/Our Blues 01.mkv` or `Reply 1988 [2015]/01 - Hand in Hand.mkv`.
/// Two- or three-digit numbers only, and a title carrying a year is left to the movie parser,
/// so `Rocky 2.mkv` and `Saga/01 - First Film (2001).mkv` stay movies.
fn single_season_episode(stem: &str, parents: &[&str]) -> Option<ParsedMedia> {
    let show = show_from_parents(parents)?;
    let (episode, title) = if let Some(c) = NUMBERED_AFTER_SHOW.captures(stem) {
        if loose(&clean_title(c.get(1)?.as_str())) != loose(&show) {
            return None;
        }
        (num(&c, 2), c.get(3).map(|m| clean_title(m.as_str())))
    } else {
        let c = NUMBERED_TITLE.captures(stem)?;
        let rest = c.get(2)?.as_str();
        if title_and_year(rest).1.is_some() {
            return None;
        }
        (num(&c, 1), Some(clean_title(rest)))
    };
    (episode > 0).then(|| ParsedMedia::Episode {
        show,
        season: 1,
        episode,
        episode_end: None,
        title: title.and_then(non_empty),
    })
}

fn parse_movie(stem: &str, parents: &[&str]) -> ParsedMedia {
    let (title, year) = title_and_year(stem);
    // Files like `movie.mkv` or `abc-1080p.mkv` inside `Title (Year)/` carry no usable name.
    if year.is_none()
        && let Some(dir) = parents.first()
        && let (dir_title, Some(dir_year)) = title_and_year(dir)
        && !dir_title.is_empty()
    {
        return ParsedMedia::Movie {
            title: dir_title,
            year: Some(dir_year),
        };
    }
    let title = if title.is_empty() {
        clean_title(stem)
    } else {
        title
    };
    ParsedMedia::Movie { title, year }
}

/// Splits `Title.2010.1080p` into the title and the last plausible year. A year at the very
/// start is part of the title (`1917`, `2001 A Space Odyssey 1968`).
fn title_and_year(name: &str) -> (String, Option<u16>) {
    // A bracketed group that is just a year (`[2021]`) is a year, not release junk: unwrap it
    // before other bracketed groups (`[1080p]`, `[GROUP]`) are dropped.
    let name = BRACKETED_YEAR.replace_all(name, " $1 ");
    let name = BRACKETED.replace_all(&name, " ");
    // Numbers in the future are part of a title ("Blade Runner 2049"), not a release year.
    let latest = current_year() + 1;
    let plausible = |m: &regex::Match<'_>| m.as_str().parse::<u16>().is_ok_and(|y| y <= latest);
    // Boundaries are checked by hand: regex has no lookaround, and consuming the separator
    // would hide adjacent years such as `_2049_2017_`.
    let is_sep = |c: Option<char>| c.is_none_or(|c| c.is_whitespace() || "._-()[]".contains(c));
    let year = YEAR
        .find_iter(&name)
        .filter(|m| m.start() > 0)
        .filter(plausible)
        .filter(|m| {
            is_sep(name[..m.start()].chars().next_back()) && is_sep(name[m.end()..].chars().next())
        })
        .last();
    match year {
        Some(m) => (clean_title(&name[..m.start()]), m.as_str().parse().ok()),
        None => (clean_title(&name), None),
    }
}

fn current_year() -> u16 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Average Gregorian year; exact enough to tell 2026 from 2049.
    (1970 + secs / 31_556_952) as u16
}

/// Normalises separators and drops release tags and anything after them.
fn clean_title(raw: &str) -> String {
    let raw = BRACKETED.replace_all(raw, " ");
    let cut = JUNK.find(&raw).map_or(raw.len(), |m| m.start());
    let kept = &raw[..cut];
    // Dots are word separators only in scene-style names; keep them in `Ms. Cobel`.
    let dots_are_separators = !kept.trim().contains(' ');
    let spaced: String = kept
        .chars()
        .map(|c| {
            if c == '_' || (c == '.' && dots_are_separators) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let collapsed = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| c.is_whitespace() || matches!(c, '-' | '(' | ')' | '[' | ']'))
        .to_string()
}

/// Season number of a season folder: `Season 2`, `S02`, or `Show S02 [1986]`.
fn season_of_dir(dir: &str) -> Option<u16> {
    SEASON_DIR
        .captures(dir)
        .or_else(|| NAMED_SEASON_DIR.captures(dir))
        .map(|c| num(&c, if c.len() == 3 { 2 } else { 1 }))
}

/// Show name in a `Show S01 …` season folder.
fn show_in_season_dir(dir: &str) -> Option<String> {
    let caps = NAMED_SEASON_DIR.captures(dir)?;
    non_empty(title_and_year(caps.get(1)?.as_str()).0)
}

/// One spelling for a show whose files name it in slightly different ways, so they group
/// together: `Bob's Burgers`, `Bob’s Burgers` and `Bobs Burgers` all become `Bobs Burgers`, and
/// `Law & Order` becomes `Law and Order`. Episodes are grouped by this name (case aside); the
/// display name normally comes from TMDB.
fn canonical_show(name: &str) -> String {
    let without_apostrophes: String = name
        .chars()
        .filter(|c| !matches!(c, '\x27' | '’' | '‘' | '`' | '´'))
        .collect();
    without_apostrophes
        .split_whitespace()
        .map(|word| if word == "&" { "and" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Letters and digits only, lowercased: `Yes, Prime Minister` and `Yes Prime Minister` agree.
fn loose(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The show folder above an episode, skipping season folders. Returns the folder's index and
/// the show name.
fn show_folder(parents: &[&str]) -> Option<(usize, String)> {
    for (ix, dir) in parents.iter().enumerate() {
        if SEASON_DIR.is_match(dir) {
            continue;
        }
        if let Some(named) = show_in_season_dir(dir) {
            // `Yes, Prime Minister/Yes Prime Minister S01/`: the parent spells the name
            // properly. Otherwise (`TV/Glee S01/`) the season folder is all there is.
            return Some(match parents.get(ix + 1) {
                Some(parent) if loose(&title_and_year(parent).0) == loose(&named) => {
                    (ix + 1, title_and_year(parent).0)
                }
                _ => (ix, named),
            });
        }
        return non_empty(title_and_year(dir).0).map(|name| (ix, name));
    }
    None
}

/// First parent folder that isn't a season folder, e.g. `Show` in `Show/Season 1/x.mkv`.
fn show_from_parents(parents: &[&str]) -> Option<String> {
    show_folder(parents).map(|(_, name)| name)
}

/// Year of the show an episode belongs to: from its folder (`TV/Shogun [2024]/Season 1/…`), a
/// season folder (`Show S01 [1986]/…`) or the release name (`Show.2026.S01E01.mkv`). Shows are
/// grouped by name only, so this is just a hint for telling remakes apart online.
pub fn show_year_hint(episode: &Path) -> Option<u16> {
    let parents: Vec<&str> = episode
        .ancestors()
        .skip(1)
        .take(4)
        .filter_map(|p| p.file_name()?.to_str())
        .collect();
    let from_folders = show_folder(&parents).and_then(|(ix, _)| {
        // The show folder first, then the season folders below it.
        parents[..=ix]
            .iter()
            .rev()
            .find_map(|dir| title_and_year(dir).1)
    });
    from_folders.or_else(|| {
        let stem = episode.file_stem()?.to_str()?;
        let at = SXXEYY.find(stem)?.start();
        title_and_year(&stem[..at]).1
    })
}

fn num(caps: &regex::Captures<'_>, group: usize) -> u16 {
    caps.get(group)
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(0)
}

fn non_empty(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(title: &str, year: Option<u16>) -> ParsedMedia {
        ParsedMedia::Movie {
            title: title.into(),
            year,
        }
    }

    fn episode(
        show: &str,
        season: u16,
        ep: u16,
        end: Option<u16>,
        title: Option<&str>,
    ) -> ParsedMedia {
        ParsedMedia::Episode {
            show: show.into(),
            season,
            episode: ep,
            episode_end: end,
            title: title.map(Into::into),
        }
    }

    fn p(path: &str) -> ParsedMedia {
        parse_path(Path::new(path))
    }

    #[test]
    fn release_style_movies() {
        assert_eq!(
            p("Inception.2010.1080p.BluRay.x264-GROUP.mkv"),
            movie("Inception", Some(2010))
        );
        assert_eq!(
            p("The Matrix (1999) [2160p] [HDR].mkv"),
            movie("The Matrix", Some(1999))
        );
        assert_eq!(
            p("Blade_Runner_2049_2017_WEB-DL.mp4"),
            movie("Blade Runner 2049", Some(2017))
        );
        assert_eq!(
            p("2001.A.Space.Odyssey.1968.REMASTERED.mkv"),
            movie("2001 A Space Odyssey", Some(1968))
        );
        assert_eq!(p("1917.mkv"), movie("1917", None));
        assert_eq!(
            p("Dune Part One [2021].mkv"),
            movie("Dune Part One", Some(2021))
        );
        assert_eq!(
            p("Blade Runner [1982].mkv"),
            movie("Blade Runner", Some(1982))
        );
        assert_eq!(
            p("Blade Runner 2049 [2017].mkv"),
            movie("Blade Runner 2049", Some(2017))
        );
        assert_eq!(p("Blade Runner 2049.mkv"), movie("Blade Runner 2049", None));
        assert_eq!(
            p("Movie [1080p] [2019] [GROUP].mkv"),
            movie("Movie", Some(2019))
        );
        assert_eq!(p("Some Home Video.mp4"), movie("Some Home Video", None));
    }

    #[test]
    fn movie_name_from_folder() {
        assert_eq!(
            p("Movies/Arrival (2016)/movie.mkv"),
            movie("Arrival", Some(2016))
        );
        assert_eq!(
            p("Movies/Dune (2021)/dune-2160p-remux.mkv"),
            movie("Dune", Some(2021))
        );
    }

    #[test]
    fn sxxeyy_episodes() {
        assert_eq!(
            p("Breaking.Bad.S01E02.Cat's.in.the.Bag.720p.HDTV.mkv"),
            episode("Breaking Bad", 1, 2, None, Some("Cat's in the Bag"))
        );
        assert_eq!(
            p("The Office - s3e10 - Benihana.mp4"),
            episode("The Office", 3, 10, None, Some("Benihana"))
        );
        assert_eq!(
            p("Show.S02E01E02.1080p.mkv"),
            episode("Show", 2, 1, Some(2), None)
        );
        assert_eq!(
            p("Show S02E01-03.mkv"),
            episode("Show", 2, 1, Some(3), None)
        );
        assert_eq!(p("Show.S01E05.720p.mkv"), episode("Show", 1, 5, None, None));
        assert_eq!(p("Firefly 1x04.avi"), episode("Firefly", 1, 4, None, None));
    }

    #[test]
    fn episodes_named_by_folders() {
        assert_eq!(
            p("TV/Severance/Season 2/S02E03.mkv"),
            episode("Severance", 2, 3, None, None)
        );
        assert_eq!(
            p("TV/Severance (2022)/Season 01/04 - The You You Are.mkv"),
            episode("Severance", 1, 4, None, Some("The You You Are"))
        );
        assert_eq!(p("TV/Andor/S1/E07.mkv"), episode("Andor", 1, 7, None, None));
        // A season folder that repeats the show name defers to the show folder above it.
        assert_eq!(
            p(
                "TV/Yes, Prime Minister/Yes Prime Minister S01 [1986]/S01E07 The Bishop's Gambit.mkv"
            ),
            episode(
                "Yes, Prime Minister",
                1,
                7,
                None,
                Some("The Bishop's Gambit")
            )
        );
        // Without a matching show folder, the season folder's name is used.
        assert_eq!(
            p("TV/Glee.S02.1080p.BluRay/S02E03.mkv"),
            episode("Glee", 2, 3, None, None)
        );
        assert_eq!(
            p("TV/Glee S02/05 - Duets.mkv"),
            episode("Glee", 2, 5, None, Some("Duets"))
        );
    }

    #[test]
    fn single_season_shows_without_season_folders() {
        assert_eq!(
            p("TV/Our Blues [2022]/Our Blues 07.mkv"),
            episode("Our Blues", 1, 7, None, None)
        );
        assert_eq!(
            p("TV/Reply 1988 [2015]/04 - Can't Help-ing.mkv"),
            episode("Reply 1988", 1, 4, None, Some("Can't Help-ing"))
        );
        assert_eq!(
            show_year_hint(Path::new("TV/Reply 1988 [2015]/01 - Hand in Hand.mkv")),
            Some(2015)
        );
        // Still movies: a single-digit sequel number, a dated title, a different name.
        assert_eq!(p("Films/Rocky/Rocky 2.mkv"), movie("Rocky 2", None));
        assert_eq!(
            p("Films/Saga/01 - First Film (2001).mkv"),
            movie("01 - First Film", Some(2001))
        );
        assert_eq!(
            p("Films/Apollo 13 (1995)/Apollo 13.mkv"),
            movie("Apollo 13", Some(1995))
        );
    }

    #[test]
    fn release_year_is_not_part_of_the_show_name() {
        assert_eq!(
            p("Dead-End Job/Dead.End.Job.2026.S01E01.2160p.NF.RGzsRutracker.mkv"),
            episode("Dead End Job", 1, 1, None, None)
        );
        assert_eq!(p("1923.S01E01.mkv"), episode("1923", 1, 1, None, None));
    }

    #[test]
    fn show_year_hint_reads_the_show_folder() {
        assert_eq!(
            show_year_hint(Path::new("TV/Shogun [2024]/Season 1/Shogun.S01E01.mkv")),
            Some(2024)
        );
        assert_eq!(
            show_year_hint(Path::new("TV/Severance (2022)/Severance.S01E01.mkv")),
            Some(2022)
        );
        assert_eq!(
            show_year_hint(Path::new("TV/Dark/Season 1/Dark.S01E01.mkv")),
            None
        );
        assert_eq!(
            show_year_hint(Path::new(
                "TV/Yes, Prime Minister/Yes Prime Minister S01 [1986]/S01E01 The Grand Design.mkv"
            )),
            Some(1986)
        );
        assert_eq!(
            show_year_hint(Path::new("Dead-End Job/Dead.End.Job.2026.S01E01.2160p.mkv")),
            Some(2026)
        );
    }

    #[test]
    fn spelling_variants_of_a_show_group_together() {
        let show_of = |path: &str| match p(path) {
            ParsedMedia::Episode { show, .. } => show,
            other => panic!("{path} parsed as {other:?}"),
        };
        // Different seasons from different releases of the same show.
        let names = [
            r"D:\TV\Bobs Burgers\Season 10\Bobs.Burgers.S10E01.1080p.WEB.h264.mkv",
            r"D:\TV\Bob's Burgers\Season 14\Bob's Burgers - S14E01 - Fight at the Not Okay Chore-al.mkv",
            r"D:\TV\Bob’s Burgers (2011)\Season 15\Bob’s Burgers (2011) - S15E03.mkv",
            r"D:\TV\Bob's Burgers\Season 16\01 - Episode.mkv",
        ];
        for name in names {
            assert_eq!(show_of(name), "Bobs Burgers", "{name}");
        }
        assert_eq!(
            show_of(r"D:\TV\Law & Order\Season 1\Law & Order - S01E01.mkv"),
            "Law and Order"
        );
        assert_eq!(show_of(r"D:\TV\Law.and.Order.S01E02.mkv"), "Law and Order");
    }

    #[test]
    fn clean_title_strips_tags() {
        assert_eq!(clean_title("Some.Movie.REPACK.x265"), "Some Movie");
        assert_eq!(clean_title("[Group] Name_Here - "), "Name Here");
        assert_eq!(clean_title(""), "");
        assert_eq!(clean_title(" - Hello, Ms. Cobel"), "Hello, Ms. Cobel");
    }
}
