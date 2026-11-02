//! The corpus: four small texts about information theory, written for this example.
//!
//! Each is shaped to exercise something. The Markdown notes have sections and paragraphs, so a
//! reading has levels to draw units from. The coding passage is added twice under two aliases,
//! which makes it one part under two manifests — the case the `identical-span` same-as engine
//! is exact for. The chat has message ids, timestamps and replies, so it has speakers to
//! pseudonymise, instants for the time engine and reply edges for ingest to draw. The draft is
//! there to be redacted.

/// Notes in Markdown: `md/1` reads sections and paragraphs.
pub const NOTES: &str = "\
# Notes on information

## Entropy

Entropy measures the average surprise of a source. A fair coin carries one bit per toss, and a \
coin that always lands heads carries none at all.

The entropy of a source is a lower bound on the average length of any lossless code for it, \
which is why a compressor can approach that bound but never beat it.

## Channels

A noisy channel has a capacity: the largest rate at which messages can cross it with an error \
probability as small as one likes, given long enough codes.

Redundancy is what lets a receiver correct errors. A code adds structured repetition, so that a \
corrupted symbol can be recovered from the symbols around it.

## History

Claude Shannon published a mathematical theory of communication in 1948, and the paper gave the \
bit its place as the unit of information.
";

/// One paragraph, added twice: once as a lecture handout and once reprinted in an anthology.
/// The same bytes are the same part, whatever the manifests say about them.
pub const CODING: &str = "\
A prefix code assigns shorter words to likelier symbols, and Huffman's construction finds the \
best such code for a known distribution; arithmetic coding goes further and spends a fraction \
of a bit where a prefix code must spend a whole one.
";

/// A paragraph the library holds for a while and then redacts.
pub const DRAFT: &str = "\
Draft: mutual information is symmetric, and it is zero exactly when the two variables are \
independent; this paragraph is a draft and will be withdrawn from the library.
";

/// The messages of the study group's chat: `(id, speaker, minute, text, replying to)`.
pub const MESSAGES: &[(u32, &str, u32, &str, Option<u32>)] = &[
    (
        1,
        "ada",
        0,
        "Is entropy the same thing as disorder, or is that only the physics meaning?",
        None,
    ),
    (
        2,
        "bruno",
        3,
        "In coding it is the average number of bits you need per symbol, nothing more mysterious.",
        Some(1),
    ),
    (
        3,
        "clare",
        7,
        "A biased coin has less entropy than a fair one, so it compresses better.",
        None,
    ),
    (
        4,
        "ada",
        12,
        "Then a channel with noise needs redundancy, or the receiver cannot correct anything.",
        Some(3),
    ),
    (
        5,
        "bruno",
        20,
        "Shannon showed the capacity is reachable with long codes, which still surprises me.",
        Some(4),
    ),
    (
        6,
        "clare",
        31,
        "I will bring the Huffman exercise tomorrow; it answers the question about message 9.",
        Some(9),
    ),
];

/// The chat as a Telegram export, which `telegram/1` reads.
///
/// Message 6 replies to message 9, which the export does not hold, so a text ingest reports it
/// (`SMY-W448`) rather than drawing an edge to nothing.
pub fn chat() -> String {
    let mut messages = Vec::new();
    for (id, who, minute, text, reply) in MESSAGES {
        let reply = reply
            .map(|to| format!(r#", "reply_to_message_id": {to}"#))
            .unwrap_or_default();
        messages.push(format!(
            r#"  {{"id": {id}, "type": "message", "date": "2024-01-15T10:{minute:02}:00",
   "date_unixtime": "{}", "from": "{who}", "from_id": "user-{who}", "text": "{text}"{reply}}}"#,
            1_705_312_800 + u64::from(*minute) * 60,
        ));
    }
    format!(
        "{{\n \"name\": \"Information theory study group\",\n \"type\": \"private_group\",\n \
         \"id\": 4242,\n \"messages\": [\n{}\n ]\n}}\n",
        messages.join(",\n")
    )
}
