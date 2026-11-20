use booknook_core::{
    imports,
    isbn_history::{self, IsbnHistoryCommand, Node},
    model::State,
    notebook_command::NotebookCommand,
    reconcile, strict_parse, validate,
};
use serde_json::{Value, json};
fn row(s: &mut State, v: &str, t: u64) {
    imports::goodreads(
        s,
        &format!("Book Id,Title,Author,ISBN\n42,Book,Author,{v}"),
        t,
    )
    .unwrap();
}
fn eid(s: &State) -> String {
    s.editions.keys().next().unwrap().clone()
}
fn vals(s: &State) -> Vec<String> {
    s.editions[&eid(s)]
        .identifiers
        .iter()
        .filter(|v| v.provider == "isbn")
        .map(|v| v.id.clone())
        .collect()
}
fn view(s: &State) -> isbn_history::IsbnHistoryView {
    isbn_history::history(s, &eid(s), "isbn").unwrap()
}
fn review(s: &mut State, v: &str) -> Value {
    let cmd = IsbnHistoryCommand::ReviewIsbn {
        edition_id: eid(s),
        namespace: "isbn".into(),
        expected_heads: view(s).heads,
        values: vec![v.into()],
    };
    cmd.apply(s).unwrap()
}
fn join(a: &State, b: &State) -> State {
    reconcile::reconcile(None, a, b).unwrap().state
}
fn initial() -> State {
    let mut s = State {
        local_reader: "synthetic".into(),
        ..State::default()
    };
    row(&mut s, "111", 10);
    s
}
fn old(s: &State) -> State {
    let mut l = s.clone();
    l.isbn_history.clear();
    l.schema = "booknook.archive.v1".into();
    l.ontology_version = "0.1.0".into();
    validate(&l).unwrap();
    l
}
#[test]
fn stale_third_peer_after_clear_and_deliberate_aba() {
    let a = initial();
    let mut b = a.clone();
    row(&mut b, "222", 9);
    let mut c = b.clone();
    row(&mut c, "", 4);
    for stale in [&a, &b, &old(&a), &old(&b)] {
        let merged = join(&c, stale);
        assert!(vals(&merged).is_empty());
        assert!(isbn_history::pending_reviews(&merged).unwrap().is_empty());
        assert_eq!(join(stale, &c).isbn_history, merged.isbn_history);
    }
    let clean = c.clone();
    row(&mut c, "111", 12);
    assert_eq!(c, clean);
    let (original, _) = view(&c)
        .nodes
        .into_iter()
        .find(|(_, n)| matches!(n,Node::Import{values,..} if values==&["111"]))
        .unwrap();
    let cmd = json!({"op":"reassert_isbn","edition_id":eid(&c),"namespace":"isbn","expected_heads":view(&c).heads,"historical_node":original});
    let apply = |s: &mut State| {
        NotebookCommand::parse(&serde_json::to_vec(&cmd).unwrap())
            .unwrap()
            .apply(s)
    };
    assert_eq!(apply(&mut c).unwrap()["replayed"], false);
    assert_eq!(vals(&c), ["111"]);
    let revision = c.revision;
    assert_eq!(apply(&mut c).unwrap()["replayed"], true);
    assert_eq!(c.revision, revision);
    assert_eq!(vals(&join(&c, &clean)), ["111"]);
}
#[test]
fn divergent_decisions_survive_three_way_sync_and_portable_restore() {
    let a = initial();
    let mut b = a.clone();
    let mut c = a.clone();
    row(&mut b, "222", 4);
    row(&mut c, "333", 99);
    let mut left = join(&b, &c);
    let mut right = join(&c, &b);
    assert_eq!(left.isbn_history, right.isbn_history);
    assert_eq!(
        isbn_history::pending_reviews(&left).unwrap()[0].choices,
        [vec!["222"], vec!["333"]]
    );
    let original_heads = view(&left).heads;
    review(&mut left, "222");
    review(&mut right, "333");
    let mut merged = join(&left, &right);
    let archive = json!({"schema":"booknook.portable-export.v1","state":merged,"assets":{}});
    let restored: booknook_core::portable_archive::PortableArchive =
        strict_parse(&serde_json::to_vec(&archive).unwrap()).unwrap();
    validate(&restored.state).unwrap();
    assert_eq!(restored.state.isbn_history, merged.isbn_history);
    let before = merged.clone();
    let cmd = IsbnHistoryCommand::ReviewIsbn {
        edition_id: eid(&merged),
        namespace: "isbn".into(),
        expected_heads: original_heads,
        values: vec!["222".into()],
    };
    // Replaying an already committed decision is acknowledged, without erasing new competing heads.
    assert_eq!(cmd.apply(&mut merged).unwrap()["replayed"], true);
    assert_eq!(before, merged);
    review(&mut merged, "333");
    assert_eq!(
        join(&join(&merged, &b), &left).isbn_history,
        merged.isbn_history
    );
}
#[test]
fn untrusted_legacy_empty_and_unrelated_namespaces() {
    let a = initial();
    let mut other = old(&a);
    let id = eid(&a);
    other
        .editions
        .get_mut(&id)
        .unwrap()
        .identifiers
        .retain(|v| v.provider != "isbn");
    let merged = join(&a, &other);
    assert_eq!(vals(&merged), ["111"]);
    assert!(view(&merged).legacy_absence_unknown);
    let mut left = a.clone();
    imports::goodreads(
        &mut left,
        "Book Id,Title,Author,ISBN13\n42,Book,Author,999",
        1,
    )
    .unwrap();
    let mut right = a.clone();
    row(&mut right, "", 3);
    let result = join(&left, &right);
    assert!(vals(&result).is_empty());
    assert!(
        result.editions[&id]
            .identifiers
            .iter()
            .any(|v| v.provider == "isbn13" && v.id == "999")
    );
    assert!(isbn_history::pending_reviews(&result).unwrap().is_empty());
}
#[test]
fn malformed_history_never_validates() {
    let a = initial();
    let id = eid(&a);
    let v = serde_json::to_value(&a).unwrap();
    for f in [
        "null",
        "wrong_version",
        "node_hash",
        "unlinked",
        "projection",
        "unknown",
        "parent",
        "self_cycle",
    ] {
        let mut j = v.clone();
        let key = view(&a).heads[0].clone();
        match f {
            "null" => j["isbn_history"] = Value::Null,
            "wrong_version" => {
                j["schema"] = json!("booknook.archive.v4");
                j["ontology_version"] = json!("0.4.0");
            }
            "node_hash" => {
                j["isbn_history"][&id]["isbn"]["nodes"][&key]["row_digest"] = json!("another")
            }
            "unlinked" => j["books"][&a.editions[&id].book_id]["sources"] = json!([]),
            "projection" => j["editions"][&id]["identifiers"] = json!([]),
            "unknown" => j["isbn_history"][&id]["isbn"]["extra"] = json!(true),
            "parent" => {
                j["isbn_history"][&id]["isbn"]["nodes"][&key]["parents"] = json!(["missing"])
            }
            "self_cycle" => j["isbn_history"][&id]["isbn"]["nodes"][&key]["parents"] = json!([key]),
            _ => unreachable!(),
        }
        assert!(
            strict_parse::<State>(&serde_json::to_vec(&j).unwrap())
                .and_then(|s| validate(&s))
                .is_err(),
            "{f}"
        );
    }
}
#[test]
fn grouped_choices_are_value_scoped_and_import_preserves_disputed_reviews() {
    fn from(s: &mut State, v: &str, conn: &str, time: u64) {
        imports::goodreads_from_connection(
            s,
            &format!("Book Id,Title,Author,ISBN\n42,Book,Author,{v}"),
            time,
            Some(conn),
        )
        .unwrap();
    }
    let a = initial();
    let mut left = a.clone();
    let mut right = a.clone();
    let mut third = a.clone();
    from(&mut left, "222", "source.left", 1);
    from(&mut right, "222", "source.right", 2);
    from(&mut third, "333", "source.third", 3);
    let agreed = join(&left, &right);
    assert_eq!(view(&agreed).heads.len(), 2);
    let mut all = join(&agreed, &third);
    assert_eq!(view(&all).heads.len(), 3);
    let mut reversed = join(&third, &join(&right, &left));
    let win = review(&mut all, "222");
    assert_eq!(win, review(&mut reversed, "222"));
    from(&mut all, "444", "source.right", 0);
    assert_eq!(vals(&all), ["444"]);
    assert_eq!(view(&all).heads.len(), 1);
    let mut competitor = join(&agreed, &third);
    review(&mut competitor, "333");
    let mut disputed = join(&all, &competitor);
    let old_heads = view(&disputed);
    from(&mut disputed, "555", "source.right", 8);
    let new = view(&disputed);
    assert_eq!(new.heads.len(), 2);
    for id in old_heads.heads {
        if matches!(old_heads.nodes[&id], Node::Review { .. }) {
            assert!(new.heads.contains(&id));
        }
    }
    assert_eq!(
        isbn_history::pending_reviews(&disputed).unwrap()[0].choices,
        [vec!["333"], vec!["555"]]
    );
}
#[test]
fn legacy_duplicate_is_noop_and_generic_conflict_cannot_overwrite_graph() {
    let modern = initial();
    let mut legacy = old(&modern);
    let before = serde_json::to_vec(&legacy).unwrap();
    row(&mut legacy, "111", 42);
    assert_eq!(serde_json::to_vec(&legacy).unwrap(), before);
    let id = eid(&modern);
    let field = serde_json::to_value(&modern.editions[&id].identifiers).unwrap();
    let conflict = reconcile::Conflict {
        id: "legacy-row".into(),
        collection: "editions".into(),
        record_id: id,
        field: "identifiers".into(),
        local: field,
        incoming: json!([]),
    };
    assert!(reconcile::resolve(&modern, &conflict, true).is_err());
    assert_eq!(
        reconcile::resolve(&modern, &conflict, false)
            .unwrap()
            .isbn_history,
        modern.isbn_history
    );
}
/// The optional private output is used to issue a frozen fixture from actual commands and hashes.
#[test]
fn generate_conformance_from_public_commands() {
    let a = initial();
    let id = eid(&a);
    let mut b = a.clone();
    let mut c = a.clone();
    row(&mut b, "222", 2);
    row(&mut c, "333", 3);
    let unresolved = join(&b, &c);
    let mut reviewed = unresolved.clone();
    let review_cmd = json!({"op":"review_isbn","edition_id":id,"namespace":"isbn","expected_heads":view(&unresolved).heads,"values":["222"]});
    let apply = |s: &mut State, cmd: &Value| {
        NotebookCommand::parse(&serde_json::to_vec(cmd).unwrap())?.apply(s)
    };
    apply(&mut reviewed, &review_cmd).unwrap();
    let hist = view(&a).heads[0].clone();
    let reassert_cmd = json!({"op":"reassert_isbn","edition_id":id,"namespace":"isbn","expected_heads":view(&reviewed).heads,"historical_node":hist});
    let mut reasserted = reviewed.clone();
    apply(&mut reasserted, &reassert_cmd).unwrap();
    let stale = json!({"op":"review_isbn","edition_id":id,"namespace":"isbn","expected_heads":view(&a).heads,"values":["222"]});
    assert!(apply(&mut reasserted, &stale).is_err());
    let v = serde_json::to_value(&unresolved).unwrap();
    let mut cases = vec![
        json!({"contract":"state","id":"legacy-untouched","input":old(&a),"valid":true}),
        json!({"contract":"state","id":"v5-unresolved","input":v,"valid":true}),
        json!({"contract":"state","id":"v5-reviewed","input":reviewed,"valid":true}),
        json!({"contract":"state","id":"v5-reasserted","input":reasserted,"valid":true}),
    ];
    let portable = json!({"schema":"booknook.portable-export.v1","state":unresolved,"assets":{}});
    booknook_core::portable_archive::parse(&serde_json::to_vec(&portable).unwrap()).unwrap();
    cases.push(
        json!({"contract":"portable","id":"v5-portable-envelope","input":portable,"valid":true}),
    );
    for (name, command, base, ok) in [
        ("review", review_cmd, &unresolved, true),
        ("reassert", reassert_cmd, &reviewed, true),
        ("stale-review", stale, &reasserted, false),
    ] {
        cases.push(
            json!({"contract":"isbn-command","id":name,"input":command,"state":base,"valid":ok}),
        );
    }
    for name in [
        "wrong-hash",
        "dangling-parent",
        "cyclic-parent",
        "projection-mismatch",
        "wrong-version",
        "null-history",
        "unknown-field",
    ] {
        let mut broken = v.clone();
        let tip = view(&unresolved).heads[0].clone();
        let nodes = &mut broken["isbn_history"][&id]["isbn"]["nodes"];
        match name {
            "wrong-hash" => nodes[&tip]["row_digest"] = json!("0".repeat(64)),
            "dangling-parent" => nodes[&tip]["parents"] = json!(["0".repeat(64)]),
            "cyclic-parent" => nodes[&tip]["parents"] = json!([tip]),
            "projection-mismatch" => broken["editions"][&id]["identifiers"] = json!([]),
            "wrong-version" => broken["ontology_version"] = json!("0.4.0"),
            "null-history" => broken["isbn_history"] = Value::Null,
            _ => broken["isbn_history"][&id]["isbn"]["future"] = json!(false),
        }
        assert!(
            strict_parse::<State>(&serde_json::to_vec(&broken).unwrap())
                .and_then(|s| validate(&s))
                .is_err()
        );
        cases.push(json!({"contract":"state","id":name,"input":broken,"valid":false}));
    }
    let raw = serde_json::to_string(&v)
        .unwrap()
        .replacen("{", "{\"isbn_history\":{},", 1);
    assert!(strict_parse::<State>(raw.as_bytes()).is_err());
    cases.push(
        json!({"contract":"state","id":"duplicate-raw-history","raw_json":raw,"valid":false}),
    );
    if let Some(path) = std::env::var_os("BOOKNOOK_GENERATE_ISBN_CONFORMANCE") {
        let output = json!({"schema":"booknook.conformance.v1","cases":cases});
        std::fs::write(path, serde_json::to_string_pretty(&output).unwrap() + "\n").unwrap();
    }
}

#[test]
fn issued_isbn_conformance_fixture_matches_runtime() {
    let fixture: Value = strict_parse(include_bytes!("fixtures/isbn-history.json")).unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    let mut names = std::collections::BTreeSet::new();
    for case in cases {
        let name = case["id"].as_str().unwrap();
        assert!(names.insert(name));
        let raw = case
            .get("raw_json")
            .map(|v| v.as_str().unwrap().as_bytes().to_vec())
            .unwrap_or_else(|| serde_json::to_vec(&case["input"]).unwrap());
        let passed = match case["contract"].as_str().unwrap() {
            "state" => strict_parse::<State>(&raw)
                .and_then(|s| validate(&s))
                .is_ok(),
            "portable" => booknook_core::portable_archive::parse(&raw).is_ok(),
            "isbn-command" => {
                let mut s: State =
                    strict_parse(&serde_json::to_vec(&case["state"]).unwrap()).unwrap();
                validate(&s).unwrap();
                let old = s.clone();
                let result = NotebookCommand::parse(&raw).and_then(|command| command.apply(&mut s));
                if result.is_err() {
                    assert_eq!(s, old, "failed fixture operation mutated state: {name}");
                }
                result.is_ok()
            }
            other => panic!("unknown ISBN fixture contract {other}"),
        };
        assert_eq!(passed, case["valid"].as_bool().unwrap(), "{name}");
    }
    assert!(
        names.contains("v5-unresolved")
            && names.contains("stale-review")
            && names.contains("duplicate-raw-history")
    );
}

#[test]
fn rehashed_imports_reject_causal_row_replay_and_overlapping_observed_parents() {
    fn retarget(node: &mut Node, old_id: &str, new_id: &str) {
        if let Node::Import { parents, .. } = node {
            assert!(parents.iter().any(|id| id == old_id));
            for id in parents.iter_mut() {
                if id == old_id {
                    *id = new_id.into();
                }
            }
            parents.sort();
        } else {
            panic!("expected import descendant");
        }
    }
    fn replace(s: &mut State, id: &str, change: impl FnOnce(&mut Node)) -> String {
        let edition = eid(s);
        let graph = &mut s
            .isbn_history
            .get_mut(&edition)
            .unwrap()
            .get_mut("isbn")
            .unwrap()
            .nodes;
        let mut n = graph.remove(id).unwrap();
        change(&mut n);
        let bytes =
            booknook_core::canonical(&("booknook.isbn-history.v1", &edition, "isbn", &n)).unwrap();
        let new_id = booknook_core::digest(&bytes);
        assert_ne!(new_id, id);
        assert!(graph.insert(new_id.clone(), n).is_none());
        new_id
    }

    let mut s = initial();
    let first = view(&s).heads[0].clone();
    // A different source row, same normalized value. Its compatibility anchor
    // is a genuine ancestor of its other observed parent, which is permitted.
    imports::goodreads(
        &mut s,
        "Book Id,Title,Author,ISBN\n42,Corrected,Author,111",
        2,
    )
    .unwrap();
    let second = view(&s).heads[0].clone();
    row(&mut s, "222", 3);
    let third = view(&s).heads[0].clone();
    row(&mut s, "333", 4);
    let last = view(&s).heads[0].clone();
    validate(&s).unwrap();
    let nodes = view(&s).nodes;
    let Node::Import {
        row_digest: first_digest,
        parents: first_parents,
        ..
    } = &nodes[&first]
    else {
        panic!()
    };
    let Node::Import {
        parents: second_parents,
        ..
    } = &nodes[&second]
    else {
        panic!()
    };
    assert!(
        second_parents.contains(&first) && first_parents.iter().all(|p| second_parents.contains(p))
    );

    let mut replay = s.clone();
    let second_new = replace(&mut replay, &second, |n| {
        let Node::Import { row_digest, .. } = n else {
            panic!()
        };
        row_digest.clone_from(first_digest);
    });
    let third_new = replace(&mut replay, &third, |n| retarget(n, &second, &second_new));
    replace(&mut replay, &last, |n| retarget(n, &third, &third_new));
    assert!(
        validate(&replay)
            .unwrap_err()
            .to_string()
            .contains("source row replayed in its own causal lineage")
    );

    let mut redundant = s;
    let third_new = replace(&mut redundant, &third, |n| {
        let Node::Import { parents, .. } = n else {
            panic!()
        };
        parents.push(first);
        parents.sort();
    });
    replace(&mut redundant, &last, |n| retarget(n, &third, &third_new));
    assert!(
        validate(&redundant)
            .unwrap_err()
            .to_string()
            .contains("observed parents overlap")
    );
}

#[test]
fn same_source_row_on_incomparable_branches_is_a_valid_agreement() {
    let a = initial();
    let mut left = a.clone();
    let mut right = a;
    row(&mut left, "222", 2);
    row(&mut right, "333", 2);
    row(&mut left, "444", 3);
    row(&mut right, "444", 9);
    let merged = join(&left, &right);
    validate(&merged).unwrap();
    assert_eq!(vals(&merged), ["444"]);
    let history = view(&merged);
    assert_eq!(history.heads.len(), 2);
    let digests: Vec<_> = history
        .heads
        .iter()
        .map(|h| {
            let Node::Import { row_digest, .. } = &history.nodes[h] else {
                panic!()
            };
            row_digest
        })
        .collect();
    assert_eq!(digests[0], digests[1]);
    assert!(isbn_history::pending_reviews(&merged).unwrap().is_empty());
}

#[test]
fn reassert_legacy_value_with_no_modern_source_observation() {
    let mut s = old(&initial());
    s.schema = "booknook.archive.v4".into();
    s.ontology_version = "0.4.0".into();
    validate(&s).unwrap();
    let legacy = s.clone();
    row(&mut s, "222", 2);
    let history = view(&s);
    let candidates: Vec<_> = history
        .nodes
        .iter()
        .filter(|(_, n)| matches!(n,Node::ValueAnchor {values} if values==&["111"]))
        .collect();
    assert_eq!(candidates.len(), 1);
    assert!(
        !history
            .nodes
            .values()
            .any(|n| matches!(n,Node::Import {values,..} if values==&["111"]))
    );
    let earlier = candidates[0].0;
    let command = |heads: Vec<String>, target: &str| IsbnHistoryCommand::ReassertIsbn {
        edition_id: history.edition_id.clone(),
        namespace: "isbn".into(),
        expected_heads: heads,
        historical_node: target.into(),
    };
    let before = s.clone();
    for invalid in [
        command(history.heads.clone(), "unknown"),
        command(history.heads.clone(), &history.heads[0]),
        command(vec![earlier.clone()], earlier),
    ] {
        assert!(invalid.apply(&mut s).is_err());
        assert_eq!(s, before);
    }
    let result = command(history.heads.clone(), earlier)
        .apply(&mut s)
        .unwrap();
    assert_eq!(result["replayed"], false);
    assert_eq!(vals(&s), ["111"]);
    let restored: State = strict_parse(&serde_json::to_vec(&s).unwrap()).unwrap();
    validate(&restored).unwrap();
    assert!(
        matches!(&view(&restored).nodes[result["node_id"].as_str().unwrap()],Node::Reassert{historical_node,..} if historical_node==earlier)
    );
    assert_eq!(join(&restored, &legacy).isbn_history, restored.isbn_history);
    assert_eq!(join(&restored, &before).isbn_history, restored.isbn_history);
    assert_eq!(
        command(history.heads, earlier).apply(&mut s).unwrap()["replayed"],
        true
    );
    assert_eq!(s, restored);
}

#[test]
fn legacy_clear_and_unknown_absence_survive_all_three_replica_merge_orders() {
    fn v4(values: &[&str]) -> State {
        let mut state = State {
            local_reader: "same-reader".into(),
            ..State::default()
        };
        for (at, value) in values.iter().enumerate() {
            row(&mut state, value, at as u64);
        }
        state.isbn_history.clear();
        state.schema = "booknook.archive.v4".into();
        state.ontology_version = "0.4.0".into();
        validate(&state).unwrap();
        state
    }

    let states = [v4(&[""]), v4(&["111", ""]), v4(&["111"])];
    assert!(!view(&states[0]).legacy_absence_unknown);
    assert!(view(&states[1]).legacy_absence_unknown);
    let mut expected = None;
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let [a, b, c] = order.map(|i| &states[i]);
        for merged in [join(&join(a, b), c), join(a, &join(b, c))] {
            let reviews = isbn_history::pending_reviews(&merged).unwrap();
            assert_eq!(
                reviews.len(),
                1,
                "lost a legacy interpretation for {order:?}"
            );
            assert_eq!(
                reviews[0].choices,
                [Vec::<String>::new(), vec!["111".to_owned()]]
            );
            assert!(view(&merged).legacy_absence_unknown);
            assert_eq!(
                expected.get_or_insert_with(|| merged.isbn_history.clone()),
                &merged.isbn_history
            );
        }
    }
}
