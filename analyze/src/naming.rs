//! `--name-routes`: ask for a name for each recently run route and write routes.txt.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;

use crate::args::Args;
use crate::dt::Date;
use crate::metrics::{Metrics, describe_route, parse_route_names, sessions_in_range};

/// A route with a session in the range, as offered for naming.
#[derive(Debug, Clone, PartialEq)]
pub struct RecentRoute {
    pub index: usize,
    /// The name it has now: from routes.txt, or else `auto`.
    pub current: String,
    pub auto: String,
    /// Sessions on the route over all history.
    pub sessions: usize,
    pub last: Date,
    /// The newest session, whose id a new name is written under.
    pub newest_id: String,
}

/// A new name for a route, or None to go back to its automatic one.
#[derive(Debug, Clone, PartialEq)]
pub struct Change {
    pub route: usize,
    pub name: Option<String>,
}

/// The routes run in the range, the most recently run first.
pub fn recent_routes(metrics: &Metrics, args: &Args) -> Vec<RecentRoute> {
    let recent: BTreeSet<usize> = sessions_in_range(metrics, args)
        .iter()
        .filter_map(|s| s.route)
        .collect();
    let mut routes: BTreeMap<usize, RecentRoute> = BTreeMap::new();
    // The sessions are oldest first, so the first one seen is the route's first track.
    for s in &metrics.sessions {
        let Some(index) = s.route.filter(|r| recent.contains(r)) else {
            continue;
        };
        let route = routes.entry(index).or_insert_with(|| RecentRoute {
            index,
            current: s.route_name.clone(),
            auto: describe_route(index, s.activity),
            sessions: 0,
            last: s.activity.date,
            newest_id: String::new(),
        });
        route.sessions += 1;
        route.last = s.activity.date;
        route.newest_id = s.activity.activity_id.clone();
    }
    let mut routes: Vec<RecentRoute> = routes.into_values().collect();
    routes.sort_by(|a, b| b.last.cmp(&a.last).then(a.index.cmp(&b.index)));
    routes
}

/// Ask for each route's name. Enter keeps it, `-` goes back to the automatic one,
/// and end of input stops with the answers so far.
pub fn ask(
    routes: &[RecentRoute],
    mut input: impl BufRead,
    mut output: impl Write,
) -> io::Result<Vec<Change>> {
    let mut changes = Vec::new();
    for route in routes {
        let plural = if route.sessions == 1 { "" } else { "s" };
        writeln!(
            output,
            "{}   {} session{plural}, last {}",
            route.auto, route.sessions, route.last
        )?;
        loop {
            write!(output, "  name [{}]: ", route.current)?;
            output.flush()?;
            let mut line = String::new();
            if input.read_line(&mut line)? == 0 {
                writeln!(output)?;
                return Ok(changes);
            }
            let answer = line.trim();
            if answer.contains('#') {
                writeln!(
                    output,
                    "  A name can't contain #, which starts a comment in routes.txt."
                )?;
                continue;
            }
            let name = match answer {
                "" => break,
                "-" => None,
                name => Some(name.to_string()),
            };
            let unchanged = match &name {
                Some(name) => *name == route.current,
                None => route.current == route.auto,
            };
            if !unchanged {
                changes.push(Change {
                    route: route.index,
                    name,
                });
            }
            break;
        }
    }
    Ok(changes)
}

/// routes.txt with the changes applied. Other lines and comments are kept; the
/// lines naming a changed route, through any of its sessions, are replaced.
pub fn rewrite(
    text: &str,
    changes: &[Change],
    metrics: &Metrics,
    routes: &[RecentRoute],
) -> String {
    let route_of: BTreeMap<&str, usize> = metrics
        .sessions
        .iter()
        .filter_map(|s| Some((s.activity.activity_id.as_str(), s.route?)))
        .collect();
    let changed: BTreeSet<usize> = changes.iter().map(|c| c.route).collect();
    let mut out = String::new();
    if text.trim().is_empty() {
        out.push_str("# Route names: <activity id> = <name>, any session of the route.\n");
    }
    for line in text.lines() {
        let names_changed = parse_route_names(line).is_ok_and(|names| {
            names.iter().any(|(id, _)| {
                route_of
                    .get(id.as_str())
                    .is_some_and(|r| changed.contains(r))
            })
        });
        if !names_changed {
            out.push_str(line);
            out.push('\n');
        }
    }
    for change in changes {
        let (Some(name), Some(route)) = (
            &change.name,
            routes.iter().find(|r| r.index == change.route),
        ) else {
            continue;
        };
        out.push_str(&format!("{} = {name}\n", route.newest_id));
    }
    out
}

/// The interactive session; returns the exit code.
pub fn run(metrics: &Metrics, args: &Args, path: &Path) -> i32 {
    if !io::stdin().is_terminal() {
        eprintln!("error: --name-routes asks questions, so it needs a terminal");
        return 1;
    }
    let routes = recent_routes(metrics, args);
    let since = args.start.map_or(String::new(), |d| format!(" since {d}"));
    if routes.is_empty() {
        println!("No routes run{since}.");
        return 0;
    }
    println!("Routes run{since} (Enter keeps, - goes back to the automatic name):");
    println!();
    let changes = match ask(&routes, io::stdin().lock(), io::stdout()) {
        Ok(changes) => changes,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    println!();
    if changes.is_empty() {
        println!("No changes.");
        return 0;
    }
    if let Err(error) = save(path, &changes, metrics, &routes) {
        eprintln!("error: {error}");
        return 1;
    }
    let plural = if changes.len() == 1 { "" } else { "s" };
    println!(
        "Wrote {} ({} name{plural} changed)",
        path.display(),
        changes.len()
    );
    0
}

/// Apply the changes to the names file, replacing it in one step.
pub fn save(
    path: &Path,
    changes: &[Change],
    metrics: &Metrics,
    routes: &[RecentRoute],
) -> Result<(), String> {
    let failed = |e: io::Error| format!("{}: {e}", path.display());
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(failed(e)),
    };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, rewrite(&text, changes, metrics, routes))
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(failed)
}
