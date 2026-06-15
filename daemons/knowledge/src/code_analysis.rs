//! CG-R4 token-free code-graph analysis: god-symbols and surprises.
//!
//! Pure graph metrics over the `CodeSymbol` call graph (the `CALLS` edges the
//! code-indexer promotes), with NO LLM and no embeddings — the AI explains on
//! top of these later (CG-R5). Two signals, both deterministic:
//!
//! - **God-symbols**: high degree-centrality nodes — a symbol that an unusual
//!   number of others call, or that calls an unusual number of others. These
//!   are the architectural hubs (a god object/function); a change to one ripples
//!   widely.
//! - **Surprises**: cross-module call edges that are the *sole* bridge between
//!   two modules. A module is a symbol's defining file (the id prefix before
//!   `#`). A lone call crossing a module boundary is an architecturally notable
//!   coupling — the kind of edge worth a second look (an unexpected dependency,
//!   a layering shortcut), surfaced token-free rather than inferred by a model.
//!
//! The functions are pure over an explicit symbol list + call-edge list, so the
//! analysis is unit-tested on small graphs without the graph engine. The
//! producer that reads the live `CodeSymbol`/`CALLS` subgraph and feeds these is
//! the wiring follow-on (it composes these with a Cypher read on the graph
//! thread); exposing the result over a socket/MCP is CG-R5.

use std::collections::BTreeMap;

use anyhow::Result;
use serde::Serialize;

use crate::graph::GraphHandle;

/// Default god-symbol degree threshold for the live-graph analysis: a symbol
/// coupled to at least this many distinct others (fan-in + fan-out) is a hub.
/// Conservative so only genuine hubs surface, not every well-used helper.
pub const DEFAULT_GOD_MIN_DEGREE: usize = 5;

/// A high-degree-centrality symbol — an architectural hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GodSymbol {
    /// The `CodeSymbol` id.
    pub id: String,
    /// How many distinct symbols call this one (fan-in).
    pub in_degree: usize,
    /// How many distinct symbols this one calls (fan-out).
    pub out_degree: usize,
}

impl GodSymbol {
    /// Total degree (fan-in + fan-out): the centrality score god-symbols rank by.
    pub fn total_degree(&self) -> usize {
        self.in_degree + self.out_degree
    }
}

/// A cross-module call that is the only edge bridging its two modules — a
/// notable architectural coupling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Surprise {
    /// The calling symbol's id.
    pub from: String,
    /// The called symbol's id.
    pub to: String,
    /// The caller's module (its defining file).
    pub from_module: String,
    /// The callee's module (its defining file).
    pub to_module: String,
}

/// The token-free analysis result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CodeAnalysis {
    /// Hubs above the degree threshold, highest centrality first.
    pub god_symbols: Vec<GodSymbol>,
    /// Sole-bridge cross-module edges.
    pub surprises: Vec<Surprise>,
}

/// The module a symbol belongs to: the id prefix before the first `#`. A
/// `CodeSymbol` id is `<file>#<kind>:<name>@<n>`, so the file path is the
/// module. An id with no `#` is its own module (the whole id), which keeps the
/// metric defined for any id shape rather than panicking on an unexpected one.
fn module_of(id: &str) -> &str {
    id.split('#').next().unwrap_or(id)
}

/// Compute the [`CodeAnalysis`] over a call graph.
///
/// `symbols` is the full node set (so a zero-degree symbol is still known) and
/// `calls` is the `CALLS` edge list as `(from_id, to_id)` pairs. A symbol is a
/// god-symbol when its total degree is at least `god_min_degree` (counting
/// DISTINCT neighbours per direction, so N parallel calls to one target are one
/// fan-out, not N). A cross-module edge is a surprise when it is the only call,
/// in either direction, between its two modules. Both lists are returned in a
/// deterministic order (god-symbols by descending total degree then id;
/// surprises by `(from, to)`), so the analysis of a fixed graph is stable.
pub fn analyze(symbols: &[String], calls: &[(String, String)], god_min_degree: usize) -> CodeAnalysis {
    // Distinct out- and in-neighbours per symbol. A `BTreeMap`/`BTreeSet` keeps
    // the iteration order deterministic without a separate sort of the keys.
    use std::collections::BTreeSet;
    let mut out: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut inc: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for s in symbols {
        out.entry(s.as_str()).or_default();
        inc.entry(s.as_str()).or_default();
    }
    // Distinct unordered module pairs and the single edge seen for a pair, so a
    // pair with exactly one crossing edge yields that edge as a surprise.
    let mut pair_count: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut pair_edge: BTreeMap<(&str, &str), (&str, &str)> = BTreeMap::new();
    for (from, to) in calls {
        let (from, to) = (from.as_str(), to.as_str());
        // A self-loop is neither a centrality signal worth flagging nor a
        // cross-module crossing; skip it so it cannot inflate a god-symbol or
        // masquerade as a module bridge.
        if from == to {
            continue;
        }
        out.entry(from).or_default().insert(to);
        inc.entry(to).or_default().insert(from);
        out.entry(to).or_default();
        inc.entry(from).or_default();

        let (fm, tm) = (module_of(from), module_of(to));
        if fm != tm {
            // Unordered module pair: a bridge is notable regardless of call
            // direction, so A->B and B->A count toward the same pair.
            let key = if fm <= tm { (fm, tm) } else { (tm, fm) };
            *pair_count.entry(key).or_default() += 1;
            pair_edge.entry(key).or_insert((from, to));
        }
    }

    let mut god_symbols: Vec<GodSymbol> = symbols
        .iter()
        .filter_map(|id| {
            let in_degree = inc.get(id.as_str()).map_or(0, BTreeSet::len);
            let out_degree = out.get(id.as_str()).map_or(0, BTreeSet::len);
            let g = GodSymbol {
                id: id.clone(),
                in_degree,
                out_degree,
            };
            (g.total_degree() >= god_min_degree && g.total_degree() > 0).then_some(g)
        })
        .collect();
    // Highest centrality first; ties broken by id so the order is stable.
    god_symbols.sort_by(|a, b| {
        b.total_degree()
            .cmp(&a.total_degree())
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut surprises: Vec<Surprise> = pair_count
        .iter()
        .filter(|(_, &count)| count == 1)
        .map(|(_, _)| ())
        .zip(pair_edge.values())
        .map(|((), &(from, to))| Surprise {
            from: from.to_string(),
            to: to.to_string(),
            from_module: module_of(from).to_string(),
            to_module: module_of(to).to_string(),
        })
        .collect();
    surprises.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.to.cmp(&b.to)));

    CodeAnalysis {
        god_symbols,
        surprises,
    }
}

/// Read the live `CodeSymbol`/`CALLS` subgraph and run [`analyze`] over it.
///
/// The full symbol set and every `CALLS` edge are read once each on the serial
/// graph thread (the `capsule_neighbors` read pattern), then the pure [`analyze`]
/// produces the result — so the graph layer only supplies the nodes and edges,
/// never the metric. Reads of an empty graph yield an empty analysis. Used by
/// the exposure step (CG-R5: a read op / `knowledge-mcp`) that surfaces the
/// hubs and surprises to the AI and the Knowledge app.
pub async fn analyze_code_graph(graph: &GraphHandle, god_min_degree: usize) -> Result<CodeAnalysis> {
    let symbols_rs = graph
        .query_rows("MATCH (s:CodeSymbol) RETURN s.id".to_string())
        .await?;
    let symbols: Vec<String> = symbols_rs
        .rows
        .iter()
        .filter_map(|row| row.first())
        .map(|c| c.as_str().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let calls_rs = graph
        .query_rows("MATCH (a:CodeSymbol)-[:CALLS]->(b:CodeSymbol) RETURN a.id, b.id".to_string())
        .await?;
    let calls: Vec<(String, String)> = calls_rs
        .rows
        .iter()
        .filter_map(|row| {
            let from = row.first()?.as_str().to_string();
            let to = row.get(1)?.as_str().to_string();
            (!from.is_empty() && !to.is_empty()).then_some((from, to))
        })
        .collect();

    Ok(analyze(&symbols, &calls, god_min_degree))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }
    fn e(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn module_of_splits_on_hash() {
        assert_eq!(module_of("/p/lib.rs#function:helper@1"), "/p/lib.rs");
        // An id with no `#` is its own module rather than panicking.
        assert_eq!(module_of("bare"), "bare");
    }

    #[test]
    fn god_symbols_rank_by_total_degree() {
        // hub is called by a, b, c and calls d: in 3, out 1, total 4.
        let symbols = s(&[
            "m.rs#fn:hub@1",
            "m.rs#fn:a@2",
            "m.rs#fn:b@3",
            "m.rs#fn:c@4",
            "m.rs#fn:d@5",
        ]);
        let calls = e(&[
            ("m.rs#fn:a@2", "m.rs#fn:hub@1"),
            ("m.rs#fn:b@3", "m.rs#fn:hub@1"),
            ("m.rs#fn:c@4", "m.rs#fn:hub@1"),
            ("m.rs#fn:hub@1", "m.rs#fn:d@5"),
        ]);
        let a = analyze(&symbols, &calls, 3);
        assert_eq!(a.god_symbols.len(), 1, "only hub clears degree 3");
        assert_eq!(a.god_symbols[0].id, "m.rs#fn:hub@1");
        assert_eq!(a.god_symbols[0].in_degree, 3);
        assert_eq!(a.god_symbols[0].out_degree, 1);
        assert_eq!(a.god_symbols[0].total_degree(), 4);
    }

    #[test]
    fn parallel_calls_count_a_neighbour_once() {
        // Two CALLS edges a->hub collapse to one distinct fan-in.
        let symbols = s(&["m.rs#fn:hub@1", "m.rs#fn:a@2"]);
        let calls = e(&[("m.rs#fn:a@2", "m.rs#fn:hub@1"), ("m.rs#fn:a@2", "m.rs#fn:hub@1")]);
        let a = analyze(&symbols, &calls, 1);
        let hub = a.god_symbols.iter().find(|g| g.id == "m.rs#fn:hub@1").unwrap();
        assert_eq!(hub.in_degree, 1, "parallel edges are one distinct neighbour");
    }

    #[test]
    fn a_sole_cross_module_edge_is_a_surprise() {
        let symbols = s(&["a.rs#fn:x@1", "b.rs#fn:y@2"]);
        let calls = e(&[("a.rs#fn:x@1", "b.rs#fn:y@2")]);
        let a = analyze(&symbols, &calls, 100);
        assert_eq!(a.surprises.len(), 1);
        assert_eq!(a.surprises[0].from, "a.rs#fn:x@1");
        assert_eq!(a.surprises[0].to, "b.rs#fn:y@2");
        assert_eq!(a.surprises[0].from_module, "a.rs");
        assert_eq!(a.surprises[0].to_module, "b.rs");
    }

    #[test]
    fn multiple_edges_between_two_modules_are_not_a_surprise() {
        // Two distinct calls bridge a.rs <-> b.rs, so the coupling is routine.
        let symbols = s(&["a.rs#fn:x@1", "a.rs#fn:z@3", "b.rs#fn:y@2", "b.rs#fn:w@4"]);
        let calls = e(&[
            ("a.rs#fn:x@1", "b.rs#fn:y@2"),
            ("a.rs#fn:z@3", "b.rs#fn:w@4"),
        ]);
        let a = analyze(&symbols, &calls, 100);
        assert!(a.surprises.is_empty(), "a well-trodden module bridge is not a surprise");
    }

    #[test]
    fn intra_module_edges_are_never_surprises() {
        let symbols = s(&["a.rs#fn:x@1", "a.rs#fn:y@2"]);
        let calls = e(&[("a.rs#fn:x@1", "a.rs#fn:y@2")]);
        assert!(analyze(&symbols, &calls, 100).surprises.is_empty());
    }

    #[test]
    fn a_self_loop_is_ignored() {
        let symbols = s(&["a.rs#fn:x@1"]);
        let calls = e(&[("a.rs#fn:x@1", "a.rs#fn:x@1")]);
        let a = analyze(&symbols, &calls, 1);
        assert!(a.god_symbols.is_empty(), "a self-loop is not a centrality signal");
        assert!(a.surprises.is_empty());
    }

    #[test]
    fn analysis_is_deterministic() {
        let symbols = s(&["a.rs#fn:x@1", "b.rs#fn:y@2", "c.rs#fn:z@3"]);
        let calls = e(&[("a.rs#fn:x@1", "b.rs#fn:y@2"), ("b.rs#fn:y@2", "c.rs#fn:z@3")]);
        let first = analyze(&symbols, &calls, 1);
        let second = analyze(&symbols, &calls, 1);
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn analyze_code_graph_reads_the_live_call_graph() {
        let tmp = tempfile::TempDir::new().unwrap();
        let graph = crate::graph::spawn(tmp.path().join("graph").to_str().unwrap()).unwrap();
        // a.rs has a hub called by p, q, r and itself calling y in b.rs — the
        // lone bridge between the two modules.
        for id in [
            "a.rs#fn:hub@1",
            "a.rs#fn:p@2",
            "a.rs#fn:q@3",
            "a.rs#fn:r@4",
            "b.rs#fn:y@5",
        ] {
            graph
                .write(format!("CREATE (:CodeSymbol {{id: '{id}', name: 'n'}})"))
                .await
                .unwrap();
        }
        for (from, to) in [
            ("a.rs#fn:p@2", "a.rs#fn:hub@1"),
            ("a.rs#fn:q@3", "a.rs#fn:hub@1"),
            ("a.rs#fn:r@4", "a.rs#fn:hub@1"),
            ("a.rs#fn:hub@1", "b.rs#fn:y@5"),
        ] {
            graph
                .write(format!(
                    "MATCH (a:CodeSymbol {{id:'{from}'}}), (b:CodeSymbol {{id:'{to}'}}) \
                     CREATE (a)-[:CALLS {{confidence:'extracted'}}]->(b)"
                ))
                .await
                .unwrap();
        }

        let a = analyze_code_graph(&graph, 3).await.unwrap();
        assert!(
            a.god_symbols.iter().any(|g| g.id == "a.rs#fn:hub@1"),
            "the hub (in 3, out 1) is a god-symbol: {:?}",
            a.god_symbols
        );
        assert_eq!(
            a.surprises.len(),
            1,
            "exactly the lone a.rs->b.rs bridge is a surprise: {:?}",
            a.surprises
        );
        assert_eq!(a.surprises[0].from, "a.rs#fn:hub@1");
        assert_eq!(a.surprises[0].to, "b.rs#fn:y@5");

        // An empty graph yields an empty analysis.
        let empty_tmp = tempfile::TempDir::new().unwrap();
        let empty = crate::graph::spawn(empty_tmp.path().join("g").to_str().unwrap()).unwrap();
        assert_eq!(
            analyze_code_graph(&empty, 1).await.unwrap(),
            CodeAnalysis::default()
        );
    }
}
