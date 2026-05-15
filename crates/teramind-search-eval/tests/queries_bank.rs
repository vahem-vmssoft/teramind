use teramind_search_eval::queries_bank::QUERIES;
use teramind_search_eval::types::QueryClass;

#[test]
fn at_least_one_hundred_queries() {
    assert!(QUERIES.len() >= 100, "got {}", QUERIES.len());
}

#[test]
fn every_class_has_at_least_twenty_queries() {
    for class in QueryClass::all() {
        let n = QUERIES.iter().filter(|q| q.class == *class).count();
        assert!(n >= 20, "class {:?} only has {} queries", class, n);
    }
}

#[test]
fn ids_are_unique() {
    let mut ids: Vec<&str> = QUERIES.iter().map(|q| q.id).collect();
    ids.sort();
    let before = ids.len();
    ids.dedup();
    assert_eq!(before, ids.len(), "duplicate ids in QUERIES");
}

#[test]
fn every_query_has_at_least_one_trigger() {
    for q in QUERIES {
        assert!(!q.triggers.is_empty(), "query {} has no triggers", q.id);
    }
}
