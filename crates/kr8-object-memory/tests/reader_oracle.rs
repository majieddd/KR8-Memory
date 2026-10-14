use kr8_object_memory::reader::{may_read, ReaderRecord, ReaderWorld, Summon};
use serde_json::Value;
use std::collections::BTreeSet;

struct Fixture(Value);
impl Fixture {
    fn strings(value: &Value) -> BTreeSet<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item.as_str().unwrap().to_owned())
            .collect()
    }
    fn profile(&self, place: &str) -> &str {
        self.0["world"]["places"][place]["profile"]
            .as_str()
            .unwrap()
    }
}
impl ReaderWorld for Fixture {
    fn readers(&self, place: &str) -> Option<BTreeSet<String>> {
        let place = self.0["world"]["places"].get(place)?;
        let profile = &self.0["world"]["profiles"][place["profile"].as_str()?];
        Some(match place["kind"].as_str()? {
            "community" => Self::strings(&profile["members"]),
            "channel" | "private" | "dm" => Self::strings(&place["members"]),
            "thread" | "canvas" => return self.readers(place["parent"].as_str()?),
            "profile-private" => {
                let mut readers = Self::strings(&profile["agents"]);
                readers.insert(profile["owner"].as_str()?.to_owned());
                readers
            }
            "soul" => BTreeSet::from([
                profile["owner"].as_str()?.to_owned(),
                place["agent"].as_str()?.to_owned(),
            ]),
            _ => return None,
        })
    }
    fn record(&self, id: &str) -> Option<ReaderRecord> {
        let record = self.0["records"]
            .as_array()?
            .iter()
            .find(|record| record["id"] == id)?;
        let place = record["place"].as_str()?;
        Some(ReaderRecord {
            profile: self.profile(place).to_owned(),
            place: place.to_owned(),
            approved_core_agent: if record["type"] == "core" {
                Some(
                    self.0["world"]["places"][place]["agent"]
                        .as_str()?
                        .to_owned(),
                )
            } else {
                None
            },
            retracted: record["status"] == "retracted",
            derived_from: record["derived_from"]
                .as_array()
                .map(|sources| {
                    sources
                        .iter()
                        .map(|source| source.as_str().unwrap().to_owned())
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

// Keep the committed 16-case Node oracle untouched. These tests bind the new
// Rust production reader rule to every existing query's independently authored labels.
#[test]
fn committed_dev_reader_labels_and_canaries_match_rust_production_rule() {
    let fixture = Fixture(
        serde_json::from_str(include_str!(
            "../../../docs/kr8/research/memory-eval/dev-v1.json"
        ))
        .unwrap(),
    );
    let queries = fixture.0["queries"].as_array().unwrap();
    assert_eq!(queries.len(), 43);
    let mut checked = 0;
    for query in queries {
        let summon = &query["summon"];
        let context = Summon {
            profile: summon["profile"].as_str().unwrap().to_owned(),
            agent: summon["agent"].as_str().unwrap().to_owned(),
            audience: fixture.readers(summon["place"].as_str().unwrap()).unwrap(),
        };
        for (label, expected) in [
            ("gold", true),
            ("acceptable", true),
            ("stale", true),
            ("decoys", true),
            ("forbidden", false),
        ] {
            for id in query[label].as_array().into_iter().flatten() {
                assert_eq!(
                    may_read(&fixture, id.as_str().unwrap(), &context),
                    expected,
                    "{} {label} {id}",
                    query["id"]
                );
                checked += 1;
            }
        }
        for canary in query["canaries"].as_array().into_iter().flatten() {
            let canary = canary.as_str().unwrap();
            for record in fixture.0["records"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|record| {
                    record["text"].as_str().unwrap().contains(canary)
                        || record["id"].as_str().unwrap().contains(canary)
                })
            {
                assert!(!may_read(
                    &fixture,
                    record["id"].as_str().unwrap(),
                    &context
                ));
            }
        }
    }
    assert!(
        checked > 70,
        "the fixture selector must exercise real labels"
    );
}

#[test]
fn unknown_source_cycles_and_missing_audience_fail_closed() {
    struct Cycle;
    impl ReaderWorld for Cycle {
        fn readers(&self, _: &str) -> Option<BTreeSet<String>> {
            Some(BTreeSet::from(["agent".into()]))
        }
        fn record(&self, id: &str) -> Option<ReaderRecord> {
            Some(ReaderRecord {
                profile: "p".into(),
                place: "place".into(),
                approved_core_agent: None,
                retracted: false,
                derived_from: vec![id.into()],
            })
        }
    }
    let summon = Summon {
        profile: "p".into(),
        agent: "agent".into(),
        audience: BTreeSet::from(["agent".into()]),
    };
    assert!(!may_read(&Cycle, "cyclic", &summon));
    assert!(!may_read(
        &Cycle,
        "cyclic",
        &Summon {
            audience: BTreeSet::new(),
            ..summon
        }
    ));
}
