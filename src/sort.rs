//! Автопорядок модов: зависимости (`require=`) идут раньше зависящих от них.
use crate::i18n::t;
use crate::mods::Mod;
use std::collections::{HashMap, HashSet};

/// Топологическая сортировка (Kahn), порядок независимых модов сохраняется как был.
/// Возвращает (новый порядок, предупреждения).
pub fn smart_sort(order: &[String], installed: &HashMap<String, Mod>) -> (Vec<String>, Vec<String>) {
    let idx: HashMap<&str, usize> = order.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let n = order.len();
    let mut deps: Vec<Vec<usize>> = vec![vec![]; n]; // deps[i] = от кого зависит i
    let mut users: Vec<Vec<usize>> = vec![vec![]; n];
    for (i, id) in order.iter().enumerate() {
        if let Some(m) = installed.get(id) {
            for r in &m.requires {
                if let Some(&j) = idx.get(r.as_str()) {
                    if j != i && !deps[i].contains(&j) {
                        deps[i].push(j);
                        users[j].push(i);
                    }
                }
            }
        }
    }
    let mut indeg: Vec<usize> = deps.iter().map(|d| d.len()).collect();
    let mut ready: std::collections::BTreeSet<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut out = Vec::with_capacity(n);
    let mut done = HashSet::new();
    while let Some(&i) = ready.iter().next() {
        ready.remove(&i);
        out.push(i);
        done.insert(i);
        for &u in &users[i] {
            indeg[u] -= 1;
            if indeg[u] == 0 {
                ready.insert(u);
            }
        }
    }
    let mut warnings = vec![];
    let rest: Vec<usize> = (0..n).filter(|i| !done.contains(i)).collect();
    if !rest.is_empty() {
        warnings.push(tf!("цикл зависимостей, оставлены в конце: {}", rest.iter().map(|&i| order[i].as_str()).collect::<Vec<_>>().join(", ")));
        out.extend(rest);
    }
    (out.into_iter().map(|i| order[i].clone()).collect(), warnings)
}
