// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_core::{
    Command, Error,
    providers::{Notebook, NotebookStorage},
};
use booknook_native::{Desk, SqliteNotebookStorage};

#[test]
fn public_sqlite_adapter_and_native_reference_app_share_the_same_command_contract() {
    let root = std::env::temp_dir().join(format!("booknook-provider-{}", booknook_core::new_id()));
    {
        let storage = SqliteNotebookStorage::open(&root).unwrap();
        let sdk = Notebook::open(storage).unwrap();
        let response = sdk
            .execute(Command::AddBook {
                title: "SDK book 🌿".into(),
                author: "SDK host".into(),
                at: 1,
            })
            .unwrap();
        let sdk_book = response.result["book_id"].as_str().unwrap();
        let native = Desk::new(root.to_string_lossy().into()).unwrap();
        let projected: booknook_core::model::State =
            serde_json::from_str(&native.snapshot().unwrap()).unwrap();
        assert_eq!(projected.books[sdk_book].title, "SDK book 🌿");
        let mut stale = sdk.snapshot().unwrap();
        let expected = stale.revision;
        native
            .execute(
                r#"{"op":"add_book","title":"Native book","author":"Native app","at":2}"#.into(),
            )
            .unwrap();
        booknook_core::apply(
            &mut stale,
            Command::AddBook {
                title: "Stale client".into(),
                author: "SDK host".into(),
                at: 3,
            },
        )
        .unwrap();
        let storage = sdk.into_storage();
        assert!(matches!(
            storage.compare_and_save(expected, &stale),
            Err(Error::Conflict(_))
        ));
        let final_state = storage.load().unwrap();
        assert_eq!(final_state.books.len(), 2);
        assert!(
            final_state
                .books
                .values()
                .any(|book| book.title == "Native book")
        );
        assert!(
            !final_state
                .books
                .values()
                .any(|book| book.title == "Stale client")
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}
