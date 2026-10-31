//! Sessions Maya hosts in a terminal of her own ("Native"): found by walking a
//! process up to one of the programs Maya started.

use std::collections::HashMap;

use crate::foreign::Proc;

/// Each process's parent, by pid: made once from a process list, then
/// walked for every session.
pub fn parents(tree: &[Proc]) -> HashMap<i32, i32> {
    tree.iter().map(|p| (p.pid, p.ppid)).collect()
}

/// The root `pid` runs under (or is), if any: walks `parents` until a pid
/// in `roots`; pid 0 or 1, an unknown parent or a loop ends the walk with `None`.
pub fn root_of(pid: i32, roots: &[i32], parents: &HashMap<i32, i32>) -> Option<i32> {
    let mut at = pid;
    // A tree that loops would never end: no walk is longer than the tree.
    for _ in 0..=parents.len() {
        if roots.contains(&at) {
            return Some(at);
        }
        if at <= 1 {
            return None;
        }
        at = *parents.get(&at)?;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: i32, ppid: i32) -> Proc {
        Proc { pid, ppid, tty: None, command: String::new() }
    }

    #[test]
    fn a_pid_under_a_root_finds_that_root() {
        let tree = parents(&[proc(10, 1), proc(20, 10), proc(30, 20)]);
        assert_eq!(root_of(30, &[10], &tree), Some(10));
        assert_eq!(root_of(10, &[10], &tree), Some(10));
    }

    #[test]
    fn a_pid_outside_every_root_or_in_a_loop_finds_none() {
        let tree = parents(&[proc(10, 1), proc(40, 41), proc(41, 40)]);
        assert_eq!(root_of(10, &[99], &tree), None);
        assert_eq!(root_of(40, &[99], &tree), None);
        assert_eq!(root_of(50, &[99], &tree), None, "an unknown pid");
    }

    #[test]
    fn the_parents_are_each_process_with_its_parent() {
        assert_eq!(parents(&[proc(10, 1), proc(20, 10)]), HashMap::from([(10, 1), (20, 10)]));
    }
}
