//! Arena-backed directory tree.
//!
//! All nodes live in one `Vec`, referenced by `NodeId` (u32). Sizes are logical
//! bytes; `on_disk` is filled only when the scan option is enabled.
//!
//! Aggregation invariant: `node.size` always includes the node's own direct file
//! bytes plus the totals of every child that has completed. A directory is
//! finalized exactly once — when it has finished enumerating its own entries
//! *and* all of its child directories have reported back — and at that moment
//! its total is added to its parent. Aggregation is therefore O(nodes) overall,
//! not O(nodes * depth), and no ancestor walk happens per file.

use std::sync::Arc;

pub type NodeId = u32;
pub const NO_NODE: NodeId = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum State {
    /// Not fully finalized yet (own enumeration or children outstanding).
    Pending,
    /// Size is final.
    Ready,
}

pub struct Node {
    pub parent: NodeId,
    pub name: Arc<str>,
    /// Logical bytes: direct files + completed child directories.
    pub size: u64,
    /// Allocated bytes; 0 unless the on-disk option was enabled for this scan.
    pub on_disk: u64,
    /// Recursive file count (files in this subtree).
    pub files: u64,
    /// Child directories still being scanned.
    pub pending: u32,
    /// This directory's own entry list has been read.
    pub enumerated: bool,
    pub is_dir: bool,
    pub reparse: bool,
    pub state: State,
    /// True once this directory's file children are present in `children`
    /// (either inserted during the scan or lazily after expanding the row).
    pub files_listed: bool,
    /// Entries that could not be read in this subtree.
    pub errors: u32,
    pub children: Vec<NodeId>,
}

pub struct Model {
    pub nodes: Vec<Node>,
    pub root: NodeId,
    /// Bumped on every structural or size change; the UI uses it to coalesce
    /// row-list rebuilds.
    pub version: u64,
}

impl Model {
    pub fn new(root_name: &str) -> Self {
        let root = Node {
            parent: NO_NODE,
            name: Arc::from(root_name),
            size: 0,
            on_disk: 0,
            files: 0,
            pending: 0,
            enumerated: false,
            is_dir: true,
            reparse: false,
            state: State::Pending,
            files_listed: false,
            errors: 0,
            children: Vec::new(),
        };
        Self {
            nodes: vec![root],
            root: 0,
            version: 1,
        }
    }

    #[inline]
    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id as usize]
    }

    /// Registers a subdirectory. The parent's `pending` counter is incremented so
    /// the parent cannot finalize before this child reports back.
    pub fn add_dir(&mut self, parent: NodeId, name: Arc<str>, reparse: bool) -> NodeId {
        let id = self.nodes.len() as NodeId;
        {
            let p = &mut self.nodes[parent as usize];
            p.children.push(id);
            p.pending += 1;
        }
        self.nodes.push(Node {
            parent,
            name,
            size: 0,
            on_disk: 0,
            files: 0,
            pending: 0,
            enumerated: false,
            is_dir: true,
            reparse,
            state: State::Pending,
            files_listed: false,
            errors: 0,
            children: Vec::new(),
        });
        self.version += 1;
        id
    }

    /// Adds a file node and immediately charges its bytes to the parent.
    pub fn add_file(&mut self, parent: NodeId, name: Arc<str>, size: u64, on_disk: u64) -> NodeId {
        let id = self.nodes.len() as NodeId;
        {
            let p = &mut self.nodes[parent as usize];
            p.children.push(id);
            p.size += size;
            p.on_disk += on_disk;
            p.files += 1;
            p.files_listed = true;
        }
        self.nodes.push(Node {
            parent,
            name,
            size,
            on_disk,
            files: 1,
            pending: 0,
            enumerated: true,
            is_dir: false,
            reparse: false,
            state: State::Ready,
            files_listed: true,
            errors: 0,
            children: Vec::new(),
        });
        self.version += 1;
        id
    }

    /// File counted without keeping a node (used when files are not displayed).
    pub fn add_file_bytes(&mut self, parent: NodeId, size: u64, on_disk: u64) {
        let p = &mut self.nodes[parent as usize];
        p.size += size;
        p.on_disk += on_disk;
        p.files += 1;
        self.version += 1;
    }

    /// Reserve the right to load this directory's file children. Returns false
    /// when they are already present, the node is not a finished directory, or a
    /// previous load claimed it (the flag is set optimistically before the
    /// background load starts).
    pub fn claim_file_listing(&mut self, id: NodeId) -> bool {
        let n = &mut self.nodes[id as usize];
        if !n.is_dir || n.reparse || n.state != State::Ready || n.files_listed {
            return false;
        }
        n.files_listed = true;
        true
    }

    /// Inserts lazily loaded file children. Their bytes were already counted by
    /// the scan (via `add_file_bytes`), so totals and counters are untouched
    /// here — only the visible child list and its sort order change.
    pub fn insert_file_children(
        &mut self,
        parent: NodeId,
        files: impl IntoIterator<Item = (Arc<str>, u64, u64)>,
    ) {
        let mut any = false;
        for (name, size, on_disk) in files {
            let id = self.nodes.len() as NodeId;
            self.nodes.push(Node {
                parent,
                name,
                size,
                on_disk,
                files: 1,
                pending: 0,
                enumerated: true,
                is_dir: false,
                reparse: false,
                state: State::Ready,
                files_listed: true,
                errors: 0,
                children: Vec::new(),
            });
            self.nodes[parent as usize].children.push(id);
            any = true;
        }
        self.nodes[parent as usize].files_listed = true;
        if any {
            self.sort_children(parent);
        }
        self.version += 1;
    }

    /// The directory's own entry list has been fully read.
    pub fn finish_dir(&mut self, id: NodeId, errors: u32) {
        let ready = {
            let n = &mut self.nodes[id as usize];
            n.errors += errors;
            n.enumerated = true;
            n.pending == 0
        };
        if ready {
            self.finalize(id);
        } else {
            self.version += 1;
        }
    }

    /// A directory whose enumeration failed: record the error, treat as empty.
    pub fn fail_dir(&mut self, id: NodeId, _code: u32) {
        self.finish_dir(id, 1);
    }

    fn finalize(&mut self, id: NodeId) {
        self.sort_children(id);
        self.nodes[id as usize].state = State::Ready;
        self.propagate(id);
    }

    fn sort_children(&mut self, id: NodeId) {
        let mut ch = std::mem::take(&mut self.nodes[id as usize].children);
        ch.sort_unstable_by(|&a, &b| {
            let na = &self.nodes[a as usize];
            let nb = &self.nodes[b as usize];
            nb.size
                .cmp(&na.size)
                .then_with(|| na.name.to_lowercase().cmp(&nb.name.to_lowercase()))
        });
        self.nodes[id as usize].children = ch;
    }

    fn propagate(&mut self, id: NodeId) {
        let mut cur = id;
        loop {
            let p = self.nodes[cur as usize].parent;
            if p == NO_NODE {
                break;
            }
            let (sz, od, fl) = {
                let n = &self.nodes[cur as usize];
                (n.size, n.on_disk, n.files)
            };
            let (done, enumerated) = {
                let pn = &mut self.nodes[p as usize];
                pn.size += sz;
                pn.on_disk += od;
                pn.files += fl;
                debug_assert!(pn.pending > 0, "pending counter underflow");
                pn.pending -= 1;
                if pn.pending == 0 && pn.enumerated {
                    pn.state = State::Ready;
                    (true, true)
                } else {
                    (pn.pending == 0, pn.enumerated)
                }
            };
            if !done || !enumerated {
                // Either children are still outstanding, or this parent has not
                // finished reading its own entries yet; it will finalize itself.
                self.version += 1;
                break;
            }
            self.sort_children(p);
            cur = p;
        }
        self.version += 1;
    }

    /// Full path of a node, reconstructed from the parent chain.
    pub fn path_of(&self, id: NodeId) -> String {
        let mut parts: Vec<&str> = Vec::new();
        let mut cur = id;
        while cur != NO_NODE && cur != self.root {
            let n = &self.nodes[cur as usize];
            parts.push(&n.name);
            cur = n.parent;
        }
        let root = &self.nodes[self.root as usize].name;
        let mut s = String::from(&**root);
        if !s.ends_with('\\') {
            s.push('\\');
        }
        for part in parts.iter().rev() {
            s.push_str(part);
            s.push('\\');
        }
        if s.len() > 3 {
            s.pop();
        }
        s
    }

    pub fn root_ready(&self) -> bool {
        let r = &self.nodes[self.root as usize];
        r.state == State::Ready && r.pending == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregation() {
        let mut m = Model::new("C:");
        let d1 = m.add_dir(m.root, Arc::from("a"), false);
        let d2 = m.add_dir(m.root, Arc::from("b"), false);
        m.add_file(d1, Arc::from("f1"), 100, 0);
        let d3 = m.add_dir(d1, Arc::from("c"), false);
        m.add_file(d3, Arc::from("f2"), 50, 0);
        m.finish_dir(d3, 0);
        m.finish_dir(d1, 0);
        m.finish_dir(d2, 0);
        m.finish_dir(m.root, 0);
        assert_eq!(m.node(d1).size, 150);
        assert_eq!(m.node(m.root).size, 150);
        assert_eq!(m.node(m.root).files, 2);
        assert!(m.root_ready());
        // sorted by size desc: a (150) before b (0)
        assert_eq!(m.node(m.root).children, vec![d1, d2]);
    }

    /// A parent that finishes enumerating before its children must not finalize
    /// early, and must still be finalized by the last child.
    #[test]
    fn out_of_order_completion() {
        let mut m = Model::new("C:");
        let a = m.add_dir(m.root, Arc::from("a"), false);
        m.finish_dir(m.root, 0);
        assert!(!m.root_ready());
        m.add_file(a, Arc::from("f"), 10, 0);
        m.finish_dir(a, 0);
        assert!(m.root_ready());
        assert_eq!(m.node(m.root).size, 10);
    }

    #[test]
    fn path_reconstruction() {
        let mut m = Model::new("C:");
        let a = m.add_dir(m.root, Arc::from("Users"), false);
        let b = m.add_dir(a, Arc::from("Public"), false);
        m.finish_dir(b, 0);
        m.finish_dir(a, 0);
        assert_eq!(m.path_of(b), "C:\\Users\\Public");
    }

    /// Lazy file rows must not change totals (their bytes were already charged)
    /// and must respect the claim gate so the UI cannot spawn duplicate loads.
    #[test]
    fn lazy_file_insert_keeps_totals() {
        let mut m = Model::new("C:");
        let d = m.add_dir(m.root, Arc::from("a"), false);
        m.add_file_bytes(d, 100, 0);
        m.add_file_bytes(d, 50, 0);
        m.finish_dir(d, 0);
        m.finish_dir(m.root, 0);
        assert_eq!(m.node(m.root).size, 150);
        assert_eq!(m.node(d).files, 2);
        assert!(m.node(d).children.is_empty());

        assert!(m.claim_file_listing(d));
        assert!(!m.claim_file_listing(d), "second claim must be refused");

        m.insert_file_children(
            d,
            vec![
                (Arc::from("small.bin"), 50, 0),
                (Arc::from("big.bin"), 100, 0),
            ],
        );
        assert_eq!(m.node(m.root).size, 150, "totals must not double count");
        assert_eq!(m.node(d).files, 2);
        let names: Vec<&str> = m
            .node(d)
            .children
            .iter()
            .map(|&c| &*m.node(c).name)
            .collect();
        assert_eq!(names, vec!["big.bin", "small.bin"], "sorted by size desc");
    }
}
