//! Conversation helpers: what gets sent to the model, and how to keep it small.
use super::db::ChatMessage;
use super::llm::{Message, Role};

/// Appended to the system prompt when concise replies are on. Output tokens
/// usually cost several times more than input tokens, so this saves the most.
pub const CONCISE_SYSTEM: &str = "Answer directly and concisely. Skip preamble, pleasantries and \
restating the question. Use the fewest words that give a complete, correct answer; use lists or code \
only when they help.";

pub fn system_prompt(user_system: &str, concise: bool) -> String {
    match (user_system.trim(), concise) {
        ("", true) => CONCISE_SYSTEM.to_string(),
        (s, true) => format!("{s}\n\n{CONCISE_SYSTEM}"),
        (s, false) => s.to_string(),
    }
}

/// Messages to send: failed and empty turns dropped, only the last `memory`
/// exchanges kept (all when `None`), same-role neighbours merged so every
/// provider accepts the sequence, and never starting with an assistant turn.
pub fn context(messages: &[ChatMessage], memory: Option<usize>) -> Vec<Message> {
    let usable: Vec<&ChatMessage> =
        messages.iter().filter(|m| m.error.is_none() && !m.content.trim().is_empty()).collect();

    let start = match memory {
        // An exchange is a user message plus the reply; count user messages from the end.
        Some(n) => usable
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, m)| m.role == Role::User)
            .nth(n.max(1) - 1)
            .map_or(0, |(i, _)| i),
        None => 0,
    };

    let mut out: Vec<Message> = Vec::new();
    for m in &usable[start..] {
        match out.last_mut() {
            Some(last) if last.role == m.role => {
                last.content.push_str("\n\n");
                last.content.push_str(&m.content);
            }
            None if m.role == Role::Assistant => {}
            _ => out.push(Message { role: m.role, content: m.content.clone() }),
        }
    }
    out
}

/// A short single-line title from the first message.
pub fn title_from(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut title: String = line.chars().take(48).collect();
    if line.chars().count() > 48 {
        title.push('…');
    }
    if title.is_empty() {
        "New chat".to_string()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: Role, content: &str) -> ChatMessage {
        ChatMessage { role, content: content.into(), ..Default::default() }
    }

    #[test]
    fn system_prompts() {
        assert_eq!(system_prompt("", false), "");
        assert_eq!(system_prompt("", true), CONCISE_SYSTEM);
        assert!(system_prompt("Be kind.", true).starts_with("Be kind.\n\n"));
    }

    #[test]
    fn context_window_and_cleanup() {
        let mut failed = msg(Role::Assistant, "");
        failed.error = Some("boom".into());
        let msgs = vec![
            msg(Role::User, "q1"),
            msg(Role::Assistant, "a1"),
            msg(Role::User, "q2"),
            failed,
            msg(Role::User, "q2 again"),
            msg(Role::Assistant, "a2"),
            msg(Role::User, "q3"),
        ];

        let all = context(&msgs, None);
        assert_eq!(all.len(), 5);
        assert_eq!(all[2].content, "q2\n\nq2 again"); // failed reply dropped, users merged

        let last = context(&msgs, Some(1));
        assert_eq!(last.len(), 1);
        assert_eq!(last[0].content, "q3");

        let two = context(&msgs, Some(2));
        assert_eq!(two[0].role, Role::User);
        assert_eq!(two[0].content, "q2 again");
        assert_eq!(two.len(), 3);

        // Never starts with an assistant turn.
        let odd = vec![msg(Role::Assistant, "hello"), msg(Role::User, "hi")];
        assert_eq!(context(&odd, None).len(), 1);
    }

    #[test]
    fn memory_edge_cases() {
        let msgs = vec![msg(Role::User, "q1"), msg(Role::Assistant, "a1"), msg(Role::User, "q2")];
        assert_eq!(context(&msgs, Some(0)).len(), 1, "zero is treated as one exchange");
        assert_eq!(context(&msgs, Some(100)).len(), 3);
        assert!(context(&[], None).is_empty());
        let only_errors = vec![ChatMessage { error: Some("x".into()), ..msg(Role::Assistant, "") }];
        assert!(context(&only_errors, None).is_empty());
    }

    #[test]
    fn titles() {
        assert_eq!(title_from("  Fix   my\nRust code "), "Fix my Rust code");
        assert!(title_from(&"x ".repeat(60)).ends_with('…'));
        assert_eq!(title_from("   "), "New chat");
    }
}
