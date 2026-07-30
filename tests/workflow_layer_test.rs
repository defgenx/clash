//! Integration tests: workflow fixtures → WorkflowRepository (FsBackend).

mod helpers;

use clash::domain::ports::WorkflowRepository;
use clash::domain::workflow::{AnnotationStatus, WorkflowStatus};
use clash::infrastructure::fs::backend::FsBackend;

use helpers::test_data_dir::TestDataDir;

#[test]
fn test_load_items_from_fixtures_skips_malformed() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    let items = backend.load_workflow_items().unwrap();
    // `broken-item` has garbage meta.json — skipped, never failing the list.
    assert_eq!(items.len(), 1);

    let item = &items[0];
    assert_eq!(item.project, "demo");
    assert_eq!(item.slug, "sample-item");
    assert_eq!(item.meta.title, "Sample auth refactor");
    assert_eq!(item.meta.status, WorkflowStatus::DiffReview);
    assert_eq!(item.meta.iteration, 2);
    assert!(item.has_plan);
    assert!(item.has_review);
    assert_eq!(item.open_annotations, 1); // one open, one addressed
    assert_eq!(item.history_iterations, vec![1]);
    assert!(item.agent_alive);

    // PR block round-trips, unknown fields preserved.
    let pr = item.meta.pr.as_ref().unwrap();
    assert_eq!(pr.number, 12);
    assert!(pr.draft);
    assert!(pr.extra.contains_key("futurePrField"));
    assert!(item.meta.extra.contains_key("someFutureField"));
}

#[test]
fn test_docs_and_annotations_read() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    let plan = backend
        .read_workflow_doc("demo", "sample-item", "plan.md")
        .unwrap();
    assert!(plan.contains("# Sample auth refactor"));

    let review = backend
        .read_workflow_doc("demo", "sample-item", "review.md")
        .unwrap();
    assert!(review.contains("## Iteration 1"));

    let anns = backend
        .load_workflow_annotations("demo", "sample-item")
        .unwrap();
    assert_eq!(anns.annotations.len(), 2);
    let open = &anns.annotations[0];
    assert_eq!(open.status, AnnotationStatus::Open);
    assert_eq!(open.line, 42);
    let addressed = &anns.annotations[1];
    assert_eq!(addressed.replies.len(), 1);
    assert!(addressed.extra.contains_key("futureAnnField"));
}

#[test]
fn test_history_diff_read() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    let diff = backend
        .read_workflow_history_diff("demo", "sample-item", 1)
        .unwrap();
    assert!(diff.contains("+fn issue_token() {}"));
    assert!(backend
        .read_workflow_history_diff("demo", "sample-item", 9)
        .is_err());
}

#[test]
fn test_full_write_cycle_over_fixture_item() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    // Snapshot the current iteration (2), then perform the request-changes
    // meta write: iteration+1 + status, exactly like the Tauri command will.
    let snapped = backend
        .snapshot_workflow_iteration("demo", "sample-item", "diff --git a/y b/y\n")
        .unwrap();
    assert_eq!(snapped, 2);

    let mut meta = backend.load_workflow_meta("demo", "sample-item").unwrap();
    meta.iteration += 1;
    meta.status = WorkflowStatus::ChangesRequested;
    backend
        .write_workflow_meta("demo", "sample-item", &meta)
        .unwrap();

    let items = backend.load_workflow_items().unwrap();
    assert_eq!(items[0].meta.iteration, 3);
    assert_eq!(items[0].meta.status, WorkflowStatus::ChangesRequested);
    assert_eq!(items[0].history_iterations, vec![1, 2]);
    // Unknown meta fields survived the read-modify-write cycle.
    assert!(items[0].meta.extra.contains_key("someFutureField"));
}

#[test]
fn test_create_item_lists_alongside_fixtures() {
    let test_dir = TestDataDir::new();
    let backend = FsBackend::new(test_dir.path.clone());

    let created = backend
        .create_workflow_item("demo", "Brand New Thing", "/tmp/repo")
        .unwrap();
    assert_eq!(created.slug, "brand-new-thing");
    assert_eq!(created.meta.status, WorkflowStatus::Draft);

    let items = backend.load_workflow_items().unwrap();
    assert_eq!(items.len(), 2);
    // Sorted by project then slug.
    assert_eq!(items[0].slug, "brand-new-thing");
    assert_eq!(items[1].slug, "sample-item");

    backend
        .delete_workflow_item("demo", "brand-new-thing")
        .unwrap();
    assert_eq!(backend.load_workflow_items().unwrap().len(), 1);
}
