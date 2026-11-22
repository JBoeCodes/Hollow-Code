//! The v1.2 quick note route over real HTTP, against a temporary vault
//! (`docs/phone-api/notes.md`).
use super::*;
use crate::phone_access::notes_api::{count_key, NoteRow, Written};
use crate::phone_access::tasks_api::{reserve_create, Reserved};

fn put(vault: &Path, relative: &str, text: &str) {
    let path = vault.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// The files `notes/` holds (none before it exists), as `notes/<name>`.
pub(super) fn note_files(vault: &Path) -> Vec<String> {
    let mut files: Vec<String> = walkdir::WalkDir::new(vault.join("notes"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| format!("notes/{}", e.file_name().to_string_lossy()))
        .collect();
    files.sort();
    files
}

/// A note file's text, by the path an answer gave.
fn text_at(vault: &Path, path: &str) -> String {
    std::fs::read_to_string(vault.join(path)).unwrap()
}

/// A request that holds only a key and the words to keep.
fn note(key: &str, text: &str) -> Value {
    json!({"idempotencyKey": key, "text": text})
}

async fn post(rig: &Rig, token: &str, body: Value) -> Resp {
    rig.call("POST", "/v1/notes", Some(token), Some(body)).await
}

/// The JSON of a 200 answer (a panic that shows the body otherwise).
fn ok(resp: &Resp) -> Value {
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    resp.json()
}

/// An error answer: its status and code, and the contract's shape, `{"error":
/// {"code", "message"}}` with one plain sentence the app may show as it is.
fn refused(resp: &Resp, status: u16, code: &str, what: &str) {
    assert_eq!(
        resp.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&resp.body)
    );
    let body = resp.json();
    assert_eq!(body["error"]["code"], code, "{what}");
    let error = body["error"].as_object().unwrap();
    assert_eq!(
        error.keys().collect::<Vec<_>>(),
        ["code", "message"],
        "{what}"
    );
    let message = str_of(&body["error"]["message"]);
    assert!(
        message.ends_with('.') && !message.contains('\n'),
        "{what}: {message}"
    );
}

#[test]
fn the_answers_serialize_to_exactly_the_fixtures() {
    let note = |disposition| Written {
        note: NoteRow {
            path: "notes/Vendor invoice question.md".into(),
            title: "Vendor invoice question".into(),
            created: "2026-11-07T09:12:44-05:00".into(),
        },
        disposition,
    };
    assert_eq!(value(&note("applied")), fixture("notes/created.json"));
    assert_eq!(value(&note("duplicate")), fixture("notes/duplicate.json"));
}

/// The day's count for notes is kept apart from the one for tasks, a slot a
/// dropped guard gives back is free again, and a kept one stays until a day
/// has passed. (Its own phone ids: the counter is shared by the whole run.)
#[test]
fn the_days_count_for_notes_is_its_own_and_a_slot_comes_back() {
    let key = count_key("ph-unit0001");
    assert_ne!(key, "ph-unit0001");
    let now = Instant::now();
    let tick = |ms| now + Duration::from_millis(ms);
    let first = reserve_create(&key, now, 2).unwrap();
    let second = reserve_create(&key, tick(1), 2).unwrap();
    assert!(reserve_create(&key, tick(2), 2).is_none(), "a third of two");
    // The same phone's tasks are counted under the bare id, apart.
    assert!(reserve_create("ph-unit0001", now, 2).is_some());
    // A guard that is dropped without keeping gives its slot back.
    drop(Reserved {
        device_id: key.clone(),
        slot: second,
        keep: false,
    });
    assert!(reserve_create(&key, tick(3), 2).is_some());
    // One that keeps its slot does not.
    drop(Reserved {
        device_id: key.clone(),
        slot: first,
        keep: true,
    });
    assert!(reserve_create(&key, tick(4), 2).is_none());
    // A day later the old ones are gone.
    let later = now + Duration::from_secs(24 * 60 * 60 + 1);
    assert!(reserve_create(&key, later, 2).is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_notes_route_needs_a_token_and_only_creates() {
    let rig = rig().await;
    let body = || Some(note("auth-key-0001", "Quiet"));
    refused(
        &rig.call("POST", "/v1/notes", None, body()).await,
        401,
        "unauthorized",
        "no token",
    );
    refused(
        &rig.call("POST", "/v1/notes", Some("not-a-paired-token"), body())
            .await,
        401,
        "unauthorized",
        "an unknown token",
    );
    assert!(note_files(&rig.vault).is_empty());
    let token = rig.token();
    // Nothing reads, edits or deletes a note, and no route names one in its path.
    for (method, path) in [
        ("GET", "/v1/notes"),
        ("PUT", "/v1/notes"),
        ("DELETE", "/v1/notes"),
        ("POST", "/v1/notes/append"),
        ("POST", "/v1/notes/delete"),
        ("DELETE", "/v1/notes/Quiet.md"),
        ("GET", "/v1/notes/notes/Quiet.md"),
    ] {
        let sent = (method != "GET").then(|| note("auth-key-0002", "Quiet"));
        let resp = rig.call(method, path, Some(&token), sent).await;
        assert_eq!(resp.status, 404, "{method} {path}");
    }
    assert!(note_files(&rig.vault).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_is_the_macs_own_file_and_one_file_however_often_the_key_is_sent() {
    let rig = rig().await;
    let (device, token) = rig.store.pair("Test iPhone", None, Utc::now()).unwrap();
    let v = &rig.vault;
    // The request fixture as it is, with its real key.
    let request = fixture("notes/create-request.json");
    let made = ok(&post(&rig, &token, request.clone()).await);
    same_shape(&made, &fixture("notes/created.json"), "created");
    assert_eq!(made["disposition"], "applied");
    assert_eq!(made["note"]["path"], "notes/Vendor invoice question.md");
    assert_eq!(made["note"]["title"], "Vendor invoice question");
    let created = str_of(&made["note"]["created"]);
    assert!(
        chrono::DateTime::parse_from_rfc3339(&created).is_ok(),
        "{created}"
    );
    // The file is what a paired device's note.create writes: its front matter
    // (the time quoted, since it holds a colon), a blank line, the words.
    assert_eq!(
        text_at(v, "notes/Vendor invoice question.md"),
        format!(
            "---\ntitle: Vendor invoice question\ncreated: \"{created}\"\nsource: \"companion: Test iPhone\"\ncompanion-op: {}.7f3a9c2e-5b1d-4e8a-9c6f-2d4b8a1e0c35\n---\n\nAsk the vendor about the second invoice\nThe PO number was 4471.\n",
            device.device_id
        )
    );
    assert_eq!(note_files(v), ["notes/Vendor invoice question.md"]);
    assert!(crate::automated_history::lease_holder(v).is_none());

    // A retry after a lost answer makes no second file. Its answer is read
    // from the file, not the clock: pin the file's time as the fixtures have
    // it, and the whole answer is the `duplicate` fixture.
    let file = v.join("notes/Vendor invoice question.md");
    let text = std::fs::read_to_string(&file).unwrap();
    let pinned = text.replace(
        &format!("created: \"{created}\""),
        "created: \"2026-11-07T09:12:44-05:00\"",
    );
    std::fs::write(&file, pinned).unwrap();
    let again = ok(&post(&rig, &token, request.clone()).await);
    assert_eq!(again, fixture("notes/duplicate.json"));
    assert_eq!(note_files(v).len(), 1);

    // The same key and title with other words is still that first note: Hollow
    // finds a repeat by its file name and its `companion-op` line, never by
    // its words, and answers the note it has. (The contract tells the app to
    // send changed words under a key of their own; a title that makes another
    // file name would not find it at all.)
    let mut changed = request.clone();
    changed["text"] = json!("Different words, same note.");
    let same_key = ok(&post(&rig, &token, changed).await);
    assert_eq!(same_key, fixture("notes/duplicate.json"));
    assert_eq!(note_files(v).len(), 1);
    assert!(text_at(v, "notes/Vendor invoice question.md").ends_with("The PO number was 4471.\n"));

    // A title that differs only in characters a file name drops makes the
    // same name, so the key finds the first note too: `duplicate`, and the new
    // words are not saved. The contract says so (Grok's read of this route
    // found the sentence promising a new file here).
    let mut punctuated = request.clone();
    punctuated["title"] = json!("Vendor invoice question!");
    punctuated["text"] = json!("Words under a title that makes the same name.");
    let same_name = ok(&post(&rig, &token, punctuated).await);
    assert_eq!(same_name["disposition"], "duplicate");
    assert_eq!(
        same_name["note"]["path"],
        "notes/Vendor invoice question.md"
    );
    assert_eq!(note_files(v).len(), 1);
    assert!(text_at(v, "notes/Vendor invoice question.md").ends_with("The PO number was 4471.\n"));

    // Another iPhone with the same key makes a note of its own.
    let other = rig.token();
    let second = ok(&post(&rig, &other, request.clone()).await);
    assert_eq!(second["disposition"], "applied");
    assert_eq!(second["note"]["path"], "notes/Vendor invoice question 2.md");
    assert_eq!(note_files(v).len(), 2);

    // A note the person renamed on the Mac before the retry is made again
    // (the contract says so): its name and its line are how it is found.
    std::fs::rename(
        v.join("notes/Vendor invoice question.md"),
        v.join("notes/Renamed on the Mac.md"),
    )
    .unwrap();
    let remade = ok(&post(&rig, &token, request).await);
    assert_eq!(remade["disposition"], "applied");
    assert_eq!(remade["note"]["path"], "notes/Vendor invoice question.md");
    assert_eq!(note_files(v).len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_title_is_the_one_given_or_the_first_line_of_the_text() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    // Blank leading lines are skipped, and the line is the title when it fits.
    let made = ok(&post(
        &rig,
        &token,
        note(
            "title-key-0001",
            "\n\n   \nGroceries for Friday\n- milk\n- eggs\n\n",
        ),
    )
    .await);
    assert_eq!(made["note"]["title"], "Groceries for Friday");
    assert_eq!(made["note"]["path"], "notes/Groceries for Friday.md");
    let saved = text_at(v, "notes/Groceries for Friday.md");
    assert!(
        saved.starts_with("---\ntitle: Groceries for Friday\ncreated: "),
        "{saved}"
    );
    // The words are kept whole, trimmed at both ends: the title line stays in them.
    assert!(
        saved.ends_with("\n---\n\nGroceries for Friday\n- milk\n- eggs\n"),
        "{saved}"
    );

    // Eighty characters stay whole; one more is cut.
    let eighty = "a".repeat(80);
    let made = ok(&post(
        &rig,
        &token,
        note("title-key-0002", &format!("{eighty}b\nrest")),
    )
    .await);
    assert_eq!(made["note"]["title"], eighty);
    assert_eq!(made["note"]["path"], format!("notes/{eighty}.md"));

    // The cut falls between characters, never inside one: a hundred
    // three-byte characters keep eighty (a title of 240 bytes), and the file
    // name, which is cut to 100 bytes, keeps 33 of them.
    let wide = "日".repeat(100);
    let made = ok(&post(
        &rig,
        &token,
        note("title-key-0003", &format!("{wide} and more\nsecond line")),
    )
    .await);
    assert_eq!(made["note"]["title"], "日".repeat(80));
    assert_eq!(
        made["note"]["path"],
        format!("notes/{}.md", "日".repeat(33))
    );
    // Four-byte characters too. A name with nothing a file name may hold is "note".
    let made = ok(&post(&rig, &token, note("title-key-0004", &"🙂".repeat(100))).await);
    assert_eq!(made["note"]["title"], "🙂".repeat(80));
    assert_eq!(made["note"]["path"], "notes/note.md");

    // A line break is a newline, a CR LF or a lone CR; tabs and runs of
    // spaces inside the title are one space.
    let made = ok(&post(
        &rig,
        &token,
        note(
            "title-key-0005",
            "\r\n\r\n  Call\t the   vendor \r\nThen the bank\rThen home",
        ),
    )
    .await);
    assert_eq!(made["note"]["title"], "Call the vendor");
    assert_eq!(made["note"]["path"], "notes/Call the vendor.md");

    // A title the request gives wins over the first line, is kept as one
    // line, and its file name drops what a name can't hold (here the colon,
    // which the front matter quotes).
    let made = ok(&post(
        &rig,
        &token,
        json!({
            "idempotencyKey": "title-key-0006",
            "title": "  Shopping:   weekend  ",
            "text": "Eggs"
        }),
    )
    .await);
    assert_eq!(made["note"]["title"], "Shopping: weekend");
    assert_eq!(made["note"]["path"], "notes/Shopping weekend.md");
    let saved = text_at(v, "notes/Shopping weekend.md");
    assert!(
        saved.starts_with("---\ntitle: \"Shopping: weekend\"\n"),
        "{saved}"
    );
    assert!(saved.ends_with("\n---\n\nEggs\n"), "{saved}");

    // Two hundred characters are the most, counted as characters, not bytes.
    let two_hundred = "é".repeat(200);
    let made = ok(&post(
        &rig,
        &token,
        json!({"idempotencyKey": "title-key-0007", "title": two_hundred, "text": "x"}),
    )
    .await);
    assert_eq!(made["note"]["title"], two_hundred);
    assert_eq!(made["note"]["path"], format!("notes/{}.md", "é".repeat(50)));

    // Another title with nothing a name can hold is "note" again, and the
    // taken name gets a number.
    let made = ok(&post(
        &rig,
        &token,
        json!({"idempotencyKey": "title-key-0008", "title": "🙂🙂", "text": "x"}),
    )
    .await);
    assert_eq!(made["note"]["title"], "🙂🙂");
    assert_eq!(made["note"]["path"], "notes/note 2.md");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_name_already_taken_gets_a_number_and_nothing_is_overwritten() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    let named = |key: &str| json!({"idempotencyKey": key, "title": "Clash", "text": "Mine"});
    let one = ok(&post(&rig, &token, named("clash-key-0001")).await);
    let two = ok(&post(&rig, &token, named("clash-key-0002")).await);
    assert_eq!(one["note"]["path"], "notes/Clash.md");
    assert_eq!(two["note"]["path"], "notes/Clash 2.md");
    let first_text = text_at(v, "notes/Clash.md");
    assert!(first_text.contains("clash-key-0001"), "{first_text}");
    // A retry of the second finds its own file past the first, which another key made.
    let again = ok(&post(&rig, &token, named("clash-key-0002")).await);
    assert_eq!(again["disposition"], "duplicate");
    assert_eq!(again["note"], two["note"]);
    assert_eq!(note_files(v).len(), 2);

    // A file the person made is never overwritten: the note goes beside it.
    put(v, "notes/Taken.md", "the person's own words");
    let taken = ok(&post(
        &rig,
        &token,
        json!({"idempotencyKey": "clash-key-0003", "title": "Taken", "text": "Mine"}),
    )
    .await);
    assert_eq!(taken["note"]["path"], "notes/Taken 2.md");
    assert_eq!(text_at(v, "notes/Taken.md"), "the person's own words");

    // So is a link, whatever it points at: a name held by a link is taken.
    put(v, "me/Identity.md", "the person's identity");
    std::os::unix::fs::symlink(v.join("me/Identity.md"), v.join("notes/Linked.md")).unwrap();
    let linked = ok(&post(
        &rig,
        &token,
        json!({"idempotencyKey": "clash-key-0004", "title": "Linked", "text": "Mine"}),
    )
    .await);
    assert_eq!(linked["note"]["path"], "notes/Linked 2.md");
    assert_eq!(text_at(v, "me/Identity.md"), "the person's identity");
    assert!(std::fs::symlink_metadata(v.join("notes/Linked.md"))
        .unwrap()
        .file_type()
        .is_symlink());
}

/// A title or a first line that names an agent's instruction file ("Claude",
/// "AGENTS", "claude.local") must never make that file: on a case-insensitive
/// disk `notes/CLAUDE.md` is what an agent that reads the folder loads as
/// instructions. The note gets " note" after the name instead, and a repeat
/// of the same key still finds it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_name_an_agent_would_load_as_instructions_gets_a_suffix() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    for (n, (given, path)) in [
        ("Claude", "notes/Claude note.md"),
        ("AGENTS", "notes/AGENTS note.md"),
        ("claude.local", "notes/claude.local note.md"),
        ("Agents.Override", "notes/Agents.Override note.md"),
        ("GEMINI", "notes/GEMINI note.md"),
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!("instruction-key-{n:04}");
        // As a given title, and again (another key) as the first line of the text.
        let titled = ok(&post(
            &rig,
            &token,
            json!({"idempotencyKey": key, "title": given, "text": "Do as I say."}),
        )
        .await);
        assert_eq!(titled["note"]["path"], path, "title {given}");
        let again = ok(&post(
            &rig,
            &token,
            json!({"idempotencyKey": key, "title": given, "text": "Do as I say."}),
        )
        .await);
        assert_eq!(again["disposition"], "duplicate", "{given}");
        assert_eq!(again["note"]["path"], path, "{given}");
        let derived = ok(&post(
            &rig,
            &token,
            note(
                &format!("derived-key-{n:04}"),
                &format!("{given}\nand more"),
            ),
        )
        .await);
        assert_ne!(
            derived["note"]["path"].as_str().unwrap().to_lowercase(),
            format!("notes/{}.md", given.to_lowercase()),
            "a first line of {given}"
        );
    }
    // None of the instruction names exists, in any case.
    for file in note_files(v) {
        let lower = file.to_lowercase();
        for name in [
            "claude",
            "claude.local",
            "agents",
            "agents.override",
            "gemini",
        ] {
            assert_ne!(lower, format!("notes/{name}.md"), "{file}");
        }
    }
    // An ordinary name that merely starts with one is untouched.
    let ordinary = ok(&post(
        &rig,
        &token,
        json!({"idempotencyKey": "ordinary-key-0001", "title": "Claude Code tips", "text": "x"}),
    )
    .await);
    assert_eq!(ordinary["note"]["path"], "notes/Claude Code tips.md");
}

/// APFS compares names by Unicode case folding, which puts the long s (U+017F)
/// on `s`: `AGENT\u{17F}.md` is `AGENTS.md` to the disk, and `to_lowercase()`
/// alone does not see it. A fresh vault, because on a disk that folds, a
/// name made earlier in another test would take the number instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_name_check_folds_look_alikes_as_the_disk_does() {
    let rig = rig().await;
    let token = rig.token();
    let long_s = ok(&post(
        &rig,
        &token,
        json!({"idempotencyKey": "longs-key-0001", "title": "AGENT\u{17F}", "text": "Do as I say."}),
    )
    .await);
    assert_eq!(long_s["note"]["path"], "notes/AGENT\u{17F} note.md");
    for file in note_files(&rig.vault) {
        assert_ne!(file.to_lowercase(), "notes/agents.md", "{file}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_text_may_be_48_kib_after_trimming_and_keeps_its_line_breaks_and_tabs() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    // Exactly 48 KiB is kept, whatever padding surrounds it.
    let limit = 48 * 1024;
    let text = format!("  \n{}\n\n", "x".repeat(limit));
    let made = ok(&post(&rig, &token, note("limit-key-0001", &text)).await);
    let saved = text_at(v, &str_of(&made["note"]["path"]));
    assert!(saved.ends_with(&format!("\n---\n\n{}\n", "x".repeat(limit))));
    // The limit is bytes, not characters: 24,576 two-byte characters fit, and
    // one byte more does not.
    let two_byte = "é".repeat(limit / 2);
    ok(&post(&rig, &token, note("limit-key-0002", &two_byte)).await);
    let over = post(
        &rig,
        &token,
        note("limit-key-0003", &format!("{two_byte}a")),
    )
    .await;
    refused(&over, 400, "bad_request", "one byte over");
    assert_eq!(note_files(v).len(), 2);

    // Line breaks (CR LF too) and tabs stay as they were sent.
    let words = "line one\r\n\tindented line\nline three";
    let made = ok(&post(&rig, &token, note("limit-key-0004", words)).await);
    assert_eq!(made["note"]["title"], "line one");
    let saved = text_at(v, &str_of(&made["note"]["path"]));
    let (_, body) = saved.split_once("\n---\n\n").unwrap();
    assert_eq!(body, format!("{words}\n"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_that_is_not_valid_is_a_400_and_writes_nothing() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    for (what, body) in [
        ("no key", json!({"text": "X"})),
        ("a null key", json!({"idempotencyKey": null, "text": "X"})),
        ("short key", json!({"idempotencyKey": "abc", "text": "X"})),
        (
            "a slash in the key",
            json!({"idempotencyKey": "abcdefgh/ijk", "text": "X"}),
        ),
        (
            "a space in the key",
            json!({"idempotencyKey": "abcdefgh ijk", "text": "X"}),
        ),
        (
            "a colon in the key",
            json!({"idempotencyKey": "abcdefgh:ijk", "text": "X"}),
        ),
        (
            "long key",
            json!({"idempotencyKey": "k".repeat(65), "text": "X"}),
        ),
        (
            "key that is a number",
            json!({"idempotencyKey": 12345678, "text": "X"}),
        ),
        ("no text", json!({"idempotencyKey": "key-00000001"})),
        (
            "null text",
            json!({"idempotencyKey": "key-00000002", "text": null}),
        ),
        (
            "empty text",
            json!({"idempotencyKey": "key-00000003", "text": ""}),
        ),
        (
            "text of spaces, tabs and line breaks",
            json!({"idempotencyKey": "key-00000004", "text": " \n\t \r\n "}),
        ),
        (
            "text that is a number",
            json!({"idempotencyKey": "key-00000005", "text": 5}),
        ),
        (
            "text one byte over 48 KiB",
            json!({"idempotencyKey": "key-00000006", "text": "x".repeat(48 * 1024 + 1)}),
        ),
        (
            "a bell in the text",
            json!({"idempotencyKey": "key-00000007", "text": "a\u{7}b"}),
        ),
        (
            "a NUL in the text",
            json!({"idempotencyKey": "key-00000008", "text": "a\u{0}b"}),
        ),
        (
            "an escape in the text",
            json!({"idempotencyKey": "key-00000009", "text": "a\u{1b}[31mred"}),
        ),
        (
            "a delete character in the text",
            json!({"idempotencyKey": "key-0000000a", "text": "a\u{7f}b"}),
        ),
        (
            "a C1 control in the text",
            json!({"idempotencyKey": "key-0000000b", "text": "a\u{9b}b"}),
        ),
        (
            "a vertical tab ending the text (trimming would hide it)",
            json!({"idempotencyKey": "key-0000000c", "text": "hello\u{b}"}),
        ),
        (
            "a next-line mark starting the text (trimming would hide it)",
            json!({"idempotencyKey": "key-0000000d", "text": "\u{85}hello"}),
        ),
        (
            "empty title",
            json!({"idempotencyKey": "key-0000000e", "title": "", "text": "X"}),
        ),
        (
            "blank title",
            json!({"idempotencyKey": "key-0000000f", "title": "   ", "text": "X"}),
        ),
        (
            "two-line title",
            json!({"idempotencyKey": "key-00000010", "title": "one\ntwo", "text": "X"}),
        ),
        (
            "a title that ends in a line break",
            json!({"idempotencyKey": "key-00000011", "title": "X\n", "text": "X"}),
        ),
        (
            "a tab in the title",
            json!({"idempotencyKey": "key-00000012", "title": "a\tb", "text": "X"}),
        ),
        (
            "a bell in the title",
            json!({"idempotencyKey": "key-00000013", "title": "a\u{7}b", "text": "X"}),
        ),
        (
            "a line separator in the title",
            json!({"idempotencyKey": "key-00000014", "title": "a\u{2028}b", "text": "X"}),
        ),
        (
            "a title of 201 characters",
            json!({"idempotencyKey": "key-00000015", "title": "t".repeat(201), "text": "X"}),
        ),
        (
            "a title of 201 two-byte characters",
            json!({"idempotencyKey": "key-00000016", "title": "é".repeat(201), "text": "X"}),
        ),
        (
            "title that is a number",
            json!({"idempotencyKey": "key-00000017", "title": 5, "text": "X"}),
        ),
        (
            "an unknown field",
            json!({"idempotencyKey": "key-00000018", "text": "X", "folder": "me"}),
        ),
        (
            "a field of the task routes",
            json!({"idempotencyKey": "key-00000019", "text": "X", "due": "2026-11-09"}),
        ),
        ("not an object", json!(["idempotencyKey", "text"])),
        ("a string", json!("Remember the milk")),
        ("null", json!(null)),
    ] {
        let resp = post(&rig, &token, body).await;
        refused(&resp, 400, "bad_request", what);
    }
    for (what, raw) in [
        ("not JSON", &b"this is not json"[..]),
        ("an empty body", &b""[..]),
        (
            "a body cut short",
            &b"{\"idempotencyKey\": \"key-0000001a\", \"text\": \"Hel"[..],
        ),
    ] {
        let resp = rig.call_raw("POST", "/v1/notes", Some(&token), raw).await;
        refused(&resp, 400, "bad_request", what);
    }
    // Nothing was written, and no folder was made for it.
    assert!(note_files(v).is_empty(), "a refused note wrote a file");
    assert!(!v.join("notes").exists());
}

/// A day's limit of notes: a runaway app can't flood the vault. A repeat that
/// only finds its earlier file is no new note, so the limit never stops it,
/// and making tasks is counted apart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_iphone_may_make_only_so_many_notes_a_day_but_a_retry_still_answers() {
    let rig = rig().await;
    let (device, token) = rig.store.pair("Counted iPhone", None, Utc::now()).unwrap();
    let v = &rig.vault;
    // A phone that made its 200 tasks today may still make notes.
    for _ in 0..200 {
        reserve_create(&device.device_id, Instant::now(), usize::MAX).unwrap();
    }
    let first = note("count-key-0001", "Made before the limit");
    ok(&post(&rig, &token, first.clone()).await);
    assert_eq!(note_files(v).len(), 1);
    // The count is in memory: seed what a day of making notes would leave (one
    // of the 200 slots is taken already, by the first note).
    let key = count_key(&device.device_id);
    for _ in 0..199 {
        reserve_create(&key, Instant::now(), usize::MAX).unwrap();
    }
    let next = note("count-key-0002", "One too many");
    let resp = post(&rig, &token, next.clone()).await;
    refused(&resp, 400, "bad_request", "past the day's count");
    assert!(
        String::from_utf8_lossy(&resp.body).contains("200 notes"),
        "{}",
        String::from_utf8_lossy(&resp.body)
    );
    assert_eq!(note_files(v).len(), 1, "a refused note wrote a file");
    // A retry of the note that did land is no new note: it answers.
    let retry = ok(&post(&rig, &token, first).await);
    assert_eq!(retry["disposition"], "duplicate");
    // Another iPhone has its own count.
    let other = rig.token();
    ok(&post(&rig, &other, next).await);
    assert_eq!(note_files(v).len(), 2);
}

/// A slot is taken just before the file is written. A write that fails after
/// that gives it back, so the day's count only holds notes that were made.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_note_that_could_not_be_written_gives_its_slot_back() {
    let rig = rig().await;
    let (device, token) = rig.store.pair("Counted iPhone", None, Utc::now()).unwrap();
    let v = &rig.vault;
    let key = count_key(&device.device_id);
    for _ in 0..199 {
        reserve_create(&key, Instant::now(), usize::MAX).unwrap();
    }
    // The name "Crowded" is taken fifty times over, by files that are not this
    // phone's, so a note of that name has nowhere to go.
    put(v, "notes/Crowded.md", "the person's own words");
    for n in 2..=50 {
        put(
            v,
            &format!("notes/Crowded {n}.md"),
            "the person's own words",
        );
    }
    let resp = post(
        &rig,
        &token,
        json!({"idempotencyKey": "crowd-key-0001", "title": "Crowded", "text": "No room"}),
    )
    .await;
    refused(&resp, 400, "bad_request", "every name taken");
    assert!(
        String::from_utf8_lossy(&resp.body).contains("too many notes named"),
        "{}",
        String::from_utf8_lossy(&resp.body)
    );
    assert_eq!(note_files(v).len(), 50);
    // The slot came back: the 200th note of the day still goes through, and
    // then the day is used up.
    ok(&post(
        &rig,
        &token,
        json!({"idempotencyKey": "crowd-key-0002", "title": "Elsewhere", "text": "Fits"}),
    )
    .await);
    let used_up = post(&rig, &token, note("crowd-key-0003", "One too many")).await;
    refused(&used_up, 400, "bad_request", "the day is used up");
    assert_eq!(note_files(v).len(), 51);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_vault_is_a_409_busy_and_the_same_body_works_when_it_is_free() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    let request = note("retry-key-0001", "Waits for the run");
    // An automated run holds the vault: nothing is written, and the answer
    // says to retry.
    let run = crate::automated_history::try_begin_run(v, "workflow", Some("held")).unwrap();
    let resp = post(&rig, &token, request.clone()).await;
    refused(&resp, 409, "busy", "a note while a run holds the vault");
    same_shape(&resp.json(), &fixture("error-busy.json"), "busy");
    assert!(note_files(v).is_empty());
    assert!(!v.join("notes").exists());
    let _ = crate::automated_history::finish_run(v, run, "success");
    // Free again: the same body goes through, and the lease is let go.
    let made = ok(&post(&rig, &token, request.clone()).await);
    assert_eq!(made["disposition"], "applied");
    assert_eq!(note_files(v).len(), 1);
    assert!(crate::automated_history::lease_holder(v).is_none());
    // A repeat while the vault is busy waits too (the same key is safe), and
    // answers `duplicate` once it is free.
    let run = crate::automated_history::try_begin_run(v, "workflow", Some("held")).unwrap();
    refused(
        &post(&rig, &token, request.clone()).await,
        409,
        "busy",
        "a repeat while a run holds the vault",
    );
    let _ = crate::automated_history::finish_run(v, run, "success");
    let again = ok(&post(&rig, &token, request).await);
    assert_eq!(again["disposition"], "duplicate");
    assert_eq!(note_files(v).len(), 1);
}

/// A link at `notes/` would take the phone's notes to another part of the
/// vault, or out of it: the route refuses, and writes nothing through it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_notes_folder_takes_no_notes_and_nothing_is_written_outside_the_vault() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    let request = note("link-key-0001", "Out of the vault");
    // A link to a folder outside the vault.
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), v.join("notes")).unwrap();
    refused(
        &post(&rig, &token, request.clone()).await,
        400,
        "bad_request",
        "a link to a folder outside the vault",
    );
    assert_eq!(
        std::fs::read_dir(outside.path()).unwrap().count(),
        0,
        "a note was written through the link"
    );
    // A link to another folder of the vault.
    std::fs::remove_file(v.join("notes")).unwrap();
    put(v, "me/area/one.md", "private");
    std::os::unix::fs::symlink(v.join("me/area"), v.join("notes")).unwrap();
    refused(
        &post(&rig, &token, request.clone()).await,
        400,
        "bad_request",
        "a link to a folder of the vault",
    );
    assert_eq!(
        std::fs::read_dir(v.join("me/area")).unwrap().count(),
        1,
        "a note was written through the link"
    );
    // A link that goes nowhere.
    std::fs::remove_file(v.join("notes")).unwrap();
    std::os::unix::fs::symlink(v.join("nowhere"), v.join("notes")).unwrap();
    refused(
        &post(&rig, &token, request).await,
        400,
        "bad_request",
        "a link that goes nowhere",
    );
    assert!(!v.join("nowhere").exists());
    // The writer refuses a link on the path by itself too, for a caller that
    // did not look first.
    for target in [
        outside.path().to_path_buf(),
        v.join("me/area"),
        v.join("nowhere"),
    ] {
        std::fs::remove_file(v.join("notes")).unwrap();
        std::os::unix::fs::symlink(&target, v.join("notes")).unwrap();
        let made = crate::companion_inbox::notes::create_for_phone(
            v,
            "Test iPhone",
            "ph-00000000.link-key-0002",
            "Direct",
            "Out of the vault",
            chrono::Local::now(),
            || Ok(()),
        );
        assert!(
            matches!(made, Err(crate::task_broker::CompanionRefusal::Refused(_))),
            "{target:?}: {made:?}"
        );
    }
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    assert_eq!(std::fs::read_dir(v.join("me/area")).unwrap().count(), 1);
}
