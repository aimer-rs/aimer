//! The first line a client sends: `hello TOKEN [NAME] [role=ROLE]`.
//!
//! The name says who is attached, and the app shows it, so it is limited to a
//! short run of plain characters: it comes from another process and must never
//! be able to bend the app's display or its logs. The role says what the
//! connection is for, which is how the app tells the tool that launched it from
//! an agent that is controlling it.

/// The longest name a client may give itself.
pub(crate) const MAX_NAME: usize = 32;

/// What a connection is for.
///
/// Being connected says little by itself: `aimer run` stays connected for as long
/// as the app runs, supervising it, whether or not anyone is controlling the app.
/// The role is how the app tells that from an agent that is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Role {
    /// An agent or script that is remote-controlling the app, or looking at it.
    /// This is what a client that never says is taken to be, so the clients that
    /// predate roles keep behaving as they did.
    #[default]
    Agent,
    /// The tool that launched the app and holds a connection to it for the whole
    /// run. Its presence says the app can be driven, not that anyone is driving it.
    Launcher,
}

impl Role {
    const LAUNCHER: &'static str = "role=launcher";
    const AGENT: &'static str = "role=agent";
}

/// What a client called itself, and what it is for, when it said hello.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Hello<'a> {
    pub(crate) token: &'a str,
    pub(crate) name: Option<&'a str>,
    pub(crate) role: Role,
}

/// Whether `name` is something a client may call itself: one to [`MAX_NAME`]
/// characters from letters, digits and `. _ : / @ + -`, so no whitespace,
/// control characters or quoting.
pub(crate) fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/@+-".contains(&b))
}

/// The hello line for `token`, naming the client when `name` is given. An agent
/// says nothing about its role, so the line is the one clients always sent.
pub(crate) fn line(token: &str, name: Option<&str>, role: Role) -> String {
    let mut line = format!("hello {token}");
    if let Some(name) = name {
        line.push(' ');
        line.push_str(name);
    }
    if role == Role::Launcher {
        line.push(' ');
        line.push_str(Role::LAUNCHER);
    }
    line
}

/// Reads a hello line. `None` when it is not one, or says more than a token, a
/// name and a role; whether the name is acceptable is [`is_valid_name`]'s to say.
///
/// A role is a word with an `=`, which no name can contain, so it needs no name
/// before it and cannot be mistaken for one.
pub(crate) fn parse(text: &str) -> Option<Hello<'_>> {
    let mut words = text.strip_prefix("hello ")?.split_ascii_whitespace().peekable();
    let token = words.next()?;
    let name = words.next_if(|word| !word.starts_with("role="));
    let role = match words.next() {
        None => Role::Agent,
        Some(Role::LAUNCHER) => Role::Launcher,
        Some(Role::AGENT) => Role::Agent,
        Some(_) => return None,
    };
    words.next().is_none().then_some(Hello { token, name, role })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello<'a>(token: &'a str, name: Option<&'a str>, role: Role) -> Option<Hello<'a>> {
        Some(Hello { token, name, role })
    }

    #[test]
    fn a_hello_with_and_without_a_name_is_read_and_is_an_agent_unless_it_says_otherwise() {
        assert_eq!(parse("hello abc"), hello("abc", None, Role::Agent));
        assert_eq!(parse("hello abc aimer-cli"), hello("abc", Some("aimer-cli"), Role::Agent));
    }

    #[test]
    fn a_hello_can_say_what_the_client_is_for() {
        assert_eq!(parse("hello abc aimer-cli role=launcher"), hello("abc", Some("aimer-cli"), Role::Launcher));
        assert_eq!(parse("hello abc aimer-cli role=agent"), hello("abc", Some("aimer-cli"), Role::Agent));
        assert_eq!(parse("hello abc role=launcher"), hello("abc", None, Role::Launcher), "a role needs no name");
    }

    #[test]
    fn anything_else_is_not_a_hello() {
        for text in [
            "", "hello", "hello ", "hi abc", "hello abc name extra", "move 1 2",
            "hello abc name role=boss", "hello abc name role=agent extra", "hello abc role=agent name",
        ] {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn lines_round_trip() {
        assert_eq!(parse(&line("t", None, Role::Agent)), hello("t", None, Role::Agent));
        assert_eq!(parse(&line("t", Some("mcp:claude"), Role::Agent)), hello("t", Some("mcp:claude"), Role::Agent));
        assert_eq!(parse(&line("t", Some("aimer-cli"), Role::Launcher)), hello("t", Some("aimer-cli"), Role::Launcher));
        assert_eq!(parse(&line("t", None, Role::Launcher)), hello("t", None, Role::Launcher));
    }

    #[test]
    fn an_agent_hello_is_the_old_hello() {
        assert_eq!(line("t", Some("n"), Role::Agent), "hello t n", "clients that never heard of roles keep working");
    }

    #[test]
    fn names_are_short_plain_and_nonempty() {
        for good in ["aimer-cli", "mcp:claude", "a", "a.b_c/d@e+f", &"x".repeat(MAX_NAME)] {
            assert!(is_valid_name(good), "{good}");
        }
        for bad in ["", "two words", "tab\t", "new\nline", "quo\"te", "back`tick", "é", &"x".repeat(MAX_NAME + 1)] {
            assert!(!is_valid_name(bad), "{bad:?}");
        }
    }
}
