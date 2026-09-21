//! Retail's own `/help`: the commands this client answers, and what one of
//! them takes. Local only -- retail's help handler asked the server nothing
//! (`LAB_005806f0`), and ACE has no `@help` of its own (its name there is
//! `@acehelp`).

use super::{command, line, Outcome, RETAIL};
use crate::{Client, Event};

/// Retail's first line, that `/` and `@` are the same way in (0x5806f8).
const NOTE: &str = "Note: You may substitute a forward slash (/) for the at symbol (@).";

/// Retail's heading over the list (0x580712).
const AVAILABLE: &str = "Available help:";

/// Retail's line pointing at the help for one name (0x5809b5).
const MORE: &str = "For more information, type @help <command>.";

/// Retail's answer to a name it had no help for (0x580a66), said in the
/// chat type it complained in rather than the 0 its help went out in.
const UNKNOWN: &str = "Unknown command";
const UNKNOWN_KIND: u32 = 0x1A;

/// Where the rest of what acswarm does is: none of it is a `/command`, and
/// a list that did not say so would read as the whole of what it can do.
const ELSEWHERE: &str = "Anything else acswarm does is a key, a panel control or a script call.";

/// The list wraps here: long enough for a dozen names, short enough to read
/// in a headless log, where nothing wraps it for us.
const WRAP: usize = 72;

/// Answer `/help`: with no word, every command this client has a row for;
/// with one, what that command takes. `Ok` either way -- retail answered a
/// name it did not know with a line of its own, not by refusing.
///
/// Retail listed help topics and left the commands to `@help commands`. The
/// rows are all the help there is here, so they are what the list holds.
pub(super) fn help(c: &mut Client, topic: &str) -> Outcome {
    // Retail took the first word and stripped the `/` or `@` a player may
    // have typed with it (0x5809dc, the strip set "/@").
    let word = topic
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .trim_matches(['/', '@']);
    if word.is_empty() {
        line(c, NOTE);
        line(c, AVAILABLE);
        for row in wrapped(&listed(), WRAP) {
            line(c, row);
        }
        line(c, MORE);
        line(c, ELSEWHERE);
        return Ok(());
    }
    let Some(cmd) = command(word) else {
        c.events.push(Event::Chat {
            text: UNKNOWN.into(),
            kind: UNKNOWN_KIND,
        });
        return Ok(());
    };
    line(c, NOTE);
    line(c, cmd.usage);
    if let Some(also) = also(cmd.names) {
        line(c, also);
    }
    Ok(())
}

/// Every command answered here, under the name retail gave it first, in
/// alphabetical order: the list is looked in, not read through.
fn listed() -> Vec<String> {
    let mut names: Vec<String> = RETAIL.iter().map(|c| format!("/{}", c.names[0])).collect();
    names.sort();
    names
}

/// `words` in lines of at most `width` characters, each word whole.
fn wrapped(words: &[String], width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in words {
        match lines.last_mut() {
            Some(last) if last.len() + 1 + word.len() <= width => {
                last.push(' ');
                last.push_str(word);
            }
            _ => lines.push(word.clone()),
        }
    }
    lines
}

/// A row's other names, the way retail's own help ended a line ("Also: @t,
/// @send, @whisper, @w"); `None` for a row with no alias.
fn also(names: &[&str]) -> Option<String> {
    let rest: Vec<String> = names.iter().skip(1).map(|n| format!("/{n}")).collect();
    (!rest.is_empty()).then(|| format!("Also: {}", rest.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit;

    /// The chat lines an action pushed.
    fn lines(c: &mut Client) -> Vec<String> {
        c.drain_events()
            .into_iter()
            .filter_map(|e| match e {
                Event::Chat { text, .. } => Some(text),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_list_holds_every_command_the_table_answers() {
        let mut c = testkit::offline_client();
        assert_eq!(help(&mut c, ""), Ok(()));
        let said = lines(&mut c);
        assert_eq!(said.first().map(String::as_str), Some(NOTE));
        assert_eq!(said.get(1).map(String::as_str), Some(AVAILABLE));
        assert_eq!(said[said.len() - 2], MORE);
        assert_eq!(said[said.len() - 1], ELSEWHERE);
        let block = said[2..said.len() - 2].join(" ");
        let named: Vec<&str> = block.split_whitespace().collect();
        assert_eq!(
            named.len(),
            RETAIL.len(),
            "one name a row, and the rows' own first names: {named:?}"
        );
        for want in ["/tell", "/lifestone", "/consent", "/help"] {
            assert!(named.contains(&want), "{want} is missing from {named:?}");
        }
        assert!(
            said[2..said.len() - 2].iter().all(|l| l.len() <= WRAP),
            "a line ran past the wrap: {said:?}"
        );
    }

    #[test]
    fn a_named_command_answers_with_what_to_type_and_its_aliases() {
        let mut c = testkit::offline_client();
        assert_eq!(help(&mut c, "tell"), Ok(()));
        assert_eq!(
            lines(&mut c),
            [NOTE, "/tell NAME, message", "Also: /t, /send, /whisper, /w"]
        );
        // The word may be typed the way it is used, and in any case.
        assert_eq!(help(&mut c, "/LS"), Ok(()));
        assert_eq!(lines(&mut c)[1], "/lifestone");
        assert_eq!(help(&mut c, "@die please"), Ok(()));
        assert_eq!(lines(&mut c), [NOTE, "/die"], "no aliases, no Also line");
    }

    #[test]
    fn a_name_with_no_row_is_answered_the_way_retail_answered_it() {
        let mut c = testkit::offline_client();
        assert_eq!(help(&mut c, "wibble"), Ok(()));
        let said = c.drain_events();
        assert_eq!(said.len(), 1);
        assert!(
            matches!(&said[0], Event::Chat { text, kind } if text == UNKNOWN && *kind == UNKNOWN_KIND),
            "{said:?}"
        );
    }

    #[test]
    fn the_list_wraps_on_whole_names() {
        let words: Vec<String> = ["/aaaa", "/bbbb", "/cccc"].map(String::from).to_vec();
        assert_eq!(wrapped(&words, 72), ["/aaaa /bbbb /cccc"]);
        assert_eq!(wrapped(&words, 11), ["/aaaa /bbbb", "/cccc"]);
        // A word longer than the width still stands whole, on its own line.
        assert_eq!(wrapped(&words, 2), ["/aaaa", "/bbbb", "/cccc"]);
        assert!(wrapped(&[], 72).is_empty());
    }
}
