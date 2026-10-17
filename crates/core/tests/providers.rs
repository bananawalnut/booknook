// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{Command, Error, Result, model::State, providers::*};
use std::cell::{Cell, RefCell};

// A host-supplied store exercises the public port independently of the native database.
struct HostStore {
    state: RefCell<State>,
    writes: Cell<usize>,
    race: Cell<bool>,
    fail: Cell<bool>,
    available: bool,
    capabilities: Vec<Capability>,
}
impl Default for HostStore {
    fn default() -> Self {
        Self {
            state: RefCell::new(State::default()),
            writes: Cell::new(0),
            race: Cell::new(false),
            fail: Cell::new(false),
            available: true,
            capabilities: vec![Capability::NotebookStorage],
        }
    }
}
impl Provider for HostStore {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: "host-owned".into(),
            capabilities: self.capabilities.clone(),
            available: self.available,
            reason: Some("The host is offline".into()),
        }
    }
}
impl NotebookStorage for HostStore {
    fn load(&self) -> Result<State> {
        Ok(self.state.borrow().clone())
    }
    fn compare_and_save(&self, expected: u64, next: &State) -> Result<()> {
        if self.fail.get() {
            return Err(Error::Unavailable("Host write rejected".into()));
        }
        let mut current = self.state.borrow_mut();
        if self.race.replace(false) {
            booknook_core::apply(&mut current, add("Other writer"))?;
        }
        if current.revision != expected {
            return Err(Error::Conflict("Host compare-and-save failed".into()));
        }
        *current = next.clone();
        self.writes.set(self.writes.get() + 1);
        Ok(())
    }
}
fn add(title: &str) -> Command {
    Command::AddBook {
        title: title.into(),
        author: "Synthetic author".into(),
        at: 1,
    }
}

#[test]
fn injected_store_runs_real_commands_and_failed_cas_never_claims_success() {
    let notebook = Notebook::open(HostStore::default()).unwrap();
    let response = notebook.execute(add("café 🌿 日本語")).unwrap();
    let id = response.result["book_id"].as_str().unwrap();
    assert_eq!(
        notebook.snapshot().unwrap().books[id].title,
        "café 🌿 日本語"
    );
    let store = notebook.into_storage();
    store.race.set(true);
    assert!(matches!(
        execute(&store, add("Rejected competing write")),
        Err(Error::Conflict(_))
    ));
    let saved = snapshot(&store).unwrap();
    assert_eq!(saved.books.len(), 2);
    assert!(
        saved
            .books
            .values()
            .any(|book| book.title == "Other writer")
    );
    assert!(
        !saved
            .books
            .values()
            .any(|book| book.title == "Rejected competing write")
    );
    store.fail.set(true);
    assert!(execute(&store, add("Failed save")).is_err());
    assert_eq!(snapshot(&store).unwrap(), saved);
    assert_eq!(store.writes.get(), 1);
}

#[test]
fn invalid_state_commands_and_missing_capabilities_never_write() {
    let store = HostStore::default();
    assert!(execute(&store, add(" ")).is_err());
    assert_eq!(store.writes.get(), 0);
    store.state.borrow_mut().ontology_version = "unknown".into();
    assert!(Notebook::open(store).is_err());
    let unavailable = HostStore {
        available: false,
        ..HostStore::default()
    };
    assert!(matches!(
        execute(&unavailable, add("Not saved")),
        Err(Error::Unavailable(_))
    ));
    assert_eq!(unavailable.writes.get(), 0);
    let catalog_only = HostStore {
        capabilities: vec![Capability::Catalog],
        ..HostStore::default()
    };
    assert!(execute(&catalog_only, add("Not saved")).is_err());
    assert_eq!(catalog_only.writes.get(), 0);
}
