//! Pure, stable repository/stack hierarchy for the popup table.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::model::PullRequest;

/// A disclosure identity survives refreshes and cannot collide across repositories.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupId {
    Repo(String),
    Stack { repo: String, number: u64 },
}

#[derive(Debug)]
pub enum Row<'a> {
    Repository {
        id: GroupId,
        repo: &'a str,
        count: usize,
    },
    Stack {
        id: GroupId,
        number: u64,
        shown: usize,
        total: u64,
    },
    PullRequest {
        pr: &'a PullRequest,
        in_stack: bool,
    },
}

/// Alphabetical repositories, with each stack at its most recent member's
/// position in the incoming activity order. Stack children run from base to tip
/// and share a single indentation level, regardless of the stack size.
pub fn rows<'a>(prs: &'a [PullRequest], collapsed: &HashSet<GroupId>) -> Vec<Row<'a>> {
    let mut repositories: BTreeMap<&str, Vec<&PullRequest>> = BTreeMap::new();
    for pr in prs {
        repositories.entry(&pr.repo).or_default().push(pr);
    }

    let mut visible = Vec::new();
    for (repo, members) in repositories {
        let id = GroupId::Repo(repo.to_owned());
        let is_collapsed = collapsed.contains(&id);
        visible.push(Row::Repository {
            id,
            repo,
            count: members.len(),
        });
        if is_collapsed {
            continue;
        }

        let mut first_in_stack = HashMap::new();
        for (index, pr) in members.iter().enumerate() {
            if let Some(stack) = pr.stack {
                first_in_stack.entry(stack.number).or_insert(index);
            }
        }
        let mut ordered: Vec<_> = members
            .into_iter()
            .enumerate()
            .map(|(index, pr)| {
                let key = pr.stack.map_or((index, 0), |stack| {
                    (first_in_stack[&stack.number], stack.position)
                });
                (key, pr)
            })
            .collect();
        ordered.sort_by_key(|(key, _)| *key);

        let mut index = 0;
        while index < ordered.len() {
            let pr = ordered[index].1;
            let Some(stack) = pr.stack else {
                visible.push(Row::PullRequest {
                    pr,
                    in_stack: false,
                });
                index += 1;
                continue;
            };
            let shown = ordered[index..]
                .iter()
                .take_while(|(_, pr)| pr.stack.is_some_and(|s| s.number == stack.number))
                .count();
            let children = &ordered[index..index + shown];
            let total = children
                .iter()
                .filter_map(|(_, pr)| pr.stack.map(|s| s.size))
                .max()
                .unwrap_or(stack.size);
            let id = GroupId::Stack {
                repo: repo.to_owned(),
                number: stack.number,
            };
            let is_collapsed = collapsed.contains(&id);
            visible.push(Row::Stack {
                id,
                number: stack.number,
                shown,
                total,
            });
            if !is_collapsed {
                for (_, pr) in children {
                    visible.push(Row::PullRequest { pr, in_stack: true });
                }
            }
            index += shown;
        }
    }
    visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CiState, ReviewDecision, StackMembership};

    fn pr(repo: &str, number: u64, stack: Option<(u64, u64, u64)>) -> PullRequest {
        PullRequest {
            repo: repo.into(),
            number,
            title: format!("PR {number}"),
            url: format!("https://github.com/{repo}/pull/{number}"),
            is_draft: false,
            ci: CiState::Success,
            decision: ReviewDecision::Approved,
            reviewers: Vec::new(),
            stack: stack.map(|(number, position, size)| StackMembership {
                number,
                position,
                size,
            }),
        }
    }

    fn labels(rows: &[Row<'_>]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Repository { repo, count, .. } => format!("repo:{repo}:{count}"),
                Row::Stack {
                    number,
                    shown,
                    total,
                    ..
                } => format!("stack:{number}:{shown}/{total}"),
                Row::PullRequest { pr, in_stack } => {
                    format!("pr:{}:{in_stack}", pr.number)
                }
            })
            .collect()
    }

    #[test]
    fn mixed_stacks_keep_dependency_order_at_their_latest_activity_position() {
        let prs = vec![
            pr("z/repo", 1, None),
            pr("a/repo", 30, Some((7, 3, 3))),
            pr("a/repo", 99, None),
            pr("a/repo", 50, Some((8, 2, 2))),
            pr("a/repo", 10, Some((7, 1, 3))),
            pr("a/repo", 40, Some((8, 1, 2))),
            pr("a/repo", 20, Some((7, 2, 3))),
        ];
        assert_eq!(
            labels(&rows(&prs, &HashSet::new())),
            [
                "repo:a/repo:6",
                "stack:7:3/3",
                "pr:10:true",
                "pr:20:true",
                "pr:30:true",
                "pr:99:false",
                "stack:8:2/2",
                "pr:40:true",
                "pr:50:true",
                "repo:z/repo:1",
                "pr:1:false",
            ]
        );
    }

    #[test]
    fn collapsed_stack_is_scoped_to_its_repository_and_keeps_its_header() {
        let prs = vec![
            pr("a/repo", 1, Some((7, 1, 1))),
            pr("b/repo", 2, Some((7, 1, 1))),
            pr("a/repo", 3, None),
        ];
        let collapsed = HashSet::from([GroupId::Stack {
            repo: "a/repo".into(),
            number: 7,
        }]);
        assert_eq!(
            labels(&rows(&prs, &collapsed)),
            [
                "repo:a/repo:2",
                "stack:7:1/1",
                "pr:3:false",
                "repo:b/repo:1",
                "stack:7:1/1",
                "pr:2:true",
            ]
        );
    }

    #[test]
    fn collapsed_repository_hides_both_stack_headers_and_prs() {
        let prs = vec![
            pr("a/repo", 1, Some((7, 1, 1))),
            pr("b/repo", 2, None),
            pr("a/repo", 3, None),
        ];
        let collapsed = HashSet::from([GroupId::Repo("a/repo".into())]);
        assert_eq!(
            labels(&rows(&prs, &collapsed)),
            ["repo:a/repo:2", "repo:b/repo:1", "pr:2:false",]
        );
    }

    #[test]
    fn partial_stack_retains_full_total() {
        let prs = vec![
            pr("a/repo", 15, Some((7, 15, 20))),
            pr("a/repo", 3, Some((7, 3, 20))),
        ];
        assert_eq!(
            labels(&rows(&prs, &HashSet::new())),
            ["repo:a/repo:2", "stack:7:2/20", "pr:3:true", "pr:15:true",]
        );
    }

    #[test]
    fn large_stacks_are_siblings_in_dependency_order() {
        let prs: Vec<_> = (1..=20)
            .rev()
            .map(|n| pr("a/repo", n, Some((7, n, 20))))
            .collect();
        let visible = rows(&prs, &HashSet::new());
        assert_eq!(visible.len(), 22);
        for (index, row) in visible[2..].iter().enumerate() {
            let Row::PullRequest { pr, in_stack } = row else {
                panic!("expected sibling PR")
            };
            assert_eq!(pr.number, index as u64 + 1);
            assert!(*in_stack);
        }
    }

    #[test]
    fn group_ids_survive_refreshed_members_and_empty_snapshots() {
        let before = vec![pr("a/repo", 1, Some((7, 1, 2)))];
        let after = vec![pr("a/repo", 2, Some((7, 2, 3)))];
        let ids = |prs: &[PullRequest]| -> Vec<GroupId> {
            rows(prs, &HashSet::new())
                .into_iter()
                .filter_map(|row| match row {
                    Row::Repository { id, .. } | Row::Stack { id, .. } => Some(id),
                    Row::PullRequest { .. } => None,
                })
                .collect()
        };
        assert_eq!(ids(&before), ids(&after));
        let collapsed = HashSet::from([GroupId::Stack {
            repo: "a/repo".into(),
            number: 7,
        }]);
        assert!(rows(&[], &collapsed).is_empty());
        assert_eq!(
            labels(&rows(&after, &collapsed)),
            ["repo:a/repo:1", "stack:7:1/3"]
        );
    }
}
