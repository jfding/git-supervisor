//! Smoke test for check-push RESULT protocol: parse stdout → merge deploy_failures.
//! Canned data mirrors `core/tests/scripts/test-check-push-result.sh` (badrepo hard fail).

use git_supervisor::merge_deploy_failures;
use git_supervisor::ops::{parse_check_push_result, CheckPushReport};
use std::collections::{HashMap, HashSet};

#[test]
fn parse_and_merge_preserves_failed_repo_across_cycles() {
    let stdout = "result\tfail\tbadrepo\n";
    let report = parse_check_push_result(stdout);
    assert_eq!(report.failed_repos, vec!["badrepo"]);

    let whitelist: HashSet<_> = ["badrepo".into()].into_iter().collect();
    let mut deploy_failures = HashMap::new();

    merge_deploy_failures(&mut deploy_failures, "local", &whitelist, &report, false);
    assert_eq!(
        deploy_failures.get("local").unwrap(),
        &HashSet::from(["badrepo".into()])
    );

    // Second watch-cycle merge with the same failure report keeps deploy_failures populated.
    merge_deploy_failures(&mut deploy_failures, "local", &whitelist, &report, false);
    assert!(
        deploy_failures
            .get("local")
            .unwrap()
            .contains("badrepo"),
        "failed repo should remain after second merge"
    );

    let report_ok = CheckPushReport::default();
    merge_deploy_failures(&mut deploy_failures, "local", &whitelist, &report_ok, false);
    assert!(deploy_failures.get("local").is_none());
}

#[test]
fn parse_and_merge_empty_result_exit_fail_marks_whitelist() {
    let report = parse_check_push_result("");
    assert!(report.failed_repos.is_empty());

    let whitelist: HashSet<_> = ["badrepo".into(), "other".into()].into_iter().collect();
    let mut deploy_failures = HashMap::new();

    merge_deploy_failures(&mut deploy_failures, "local", &whitelist, &report, true);
    assert_eq!(deploy_failures.get("local").unwrap().len(), 2);

    merge_deploy_failures(&mut deploy_failures, "local", &whitelist, &report, true);
    assert_eq!(deploy_failures.get("local").unwrap().len(), 2);
}
