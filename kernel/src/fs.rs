//! Простая RAM-файловая система с иерархией каталогов.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeKind {
    File,
    Directory,
}

#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub kind: NodeKind,
    pub size: usize,
}

struct Node {
    name: String,
    kind: NodeKind,
    data: Vec<u8>,
    children: Vec<Node>,
}

impl Node {
    fn dir(name: &str) -> Self {
        Self {
            name: name.to_string(),
            kind: NodeKind::Directory,
            data: Vec::new(),
            children: Vec::new(),
        }
    }
    fn file(name: &str, data: &[u8]) -> Self {
        Self {
            name: name.to_string(),
            kind: NodeKind::File,
            data: data.to_vec(),
            children: Vec::new(),
        }
    }
}

pub struct FileSystem {
    root: Node,
}

impl FileSystem {
    pub fn new() -> Self {
        let mut root = Node::dir("");

        let mut desktop = Node::dir("Desktop");
        desktop
            .children
            .push(Node::file("Welcome.txt", b"Welcome to Rust OS!\n"));

        let mut documents = Node::dir("Documents");
        documents
            .children
            .push(Node::file("Notes.txt", b"This file is stored in RAMFS.\n"));
        documents
            .children
            .push(Node::file("todo.txt", b"- write kernel\n- add GUI\n- ship v1.0\n"));

        let mut system = Node::dir("System");
        system
            .children
            .push(Node::file("version.txt", b"Rust OS 0.6\n"));

        root.children.push(desktop);
        root.children.push(documents);
        root.children.push(Node::dir("Downloads"));
        root.children.push(system);

        Self { root }
    }

    fn parts(path: &str) -> impl Iterator<Item = &str> {
        path.split('/').filter(|p| !p.is_empty())
    }

    fn node(&self, path: &str) -> Option<&Node> {
        let mut node = &self.root;
        for part in Self::parts(path) {
            node = node.children.iter().find(|n| n.name == part)?;
        }
        Some(node)
    }

    fn node_mut(&mut self, path: &str) -> Option<&mut Node> {
        let mut node = &mut self.root;
        for part in Self::parts(path) {
            let i = node.children.iter().position(|n| n.name == part)?;
            node = &mut node.children[i];
        }
        Some(node)
    }

    pub fn exists(&self, path: &str) -> bool {
        self.node(path).is_some()
    }

    pub fn is_dir(&self, path: &str) -> bool {
        matches!(self.node(path), Some(n) if n.kind == NodeKind::Directory)
    }

    pub fn list(&self, path: &str) -> Vec<DirEntry> {
        let Some(node) = self.node(path) else {
            return Vec::new();
        };
        if node.kind != NodeKind::Directory {
            return Vec::new();
        }
        let mut v: Vec<DirEntry> = node
            .children
            .iter()
            .map(|n| DirEntry {
                name: n.name.clone(),
                kind: n.kind,
                size: n.data.len(),
            })
            .collect();
        // Сначала каталоги, потом файлы, внутри — по алфавиту.
        v.sort_by(|a, b| {
            let ka = (a.kind != NodeKind::Directory) as u8;
            let kb = (b.kind != NodeKind::Directory) as u8;
            ka.cmp(&kb).then_with(|| a.name.cmp(&b.name))
        });
        v
    }

    pub fn mkdir(&mut self, parent: &str, name: &str) -> bool {
        if !Self::valid_name(name) {
            return false;
        }
        let Some(p) = self.node_mut(parent) else {
            return false;
        };
        if p.kind != NodeKind::Directory {
            return false;
        }
        if p.children.iter().any(|n| n.name == name) {
            return false;
        }
        p.children.push(Node::dir(name));
        true
    }

    pub fn create_file(&mut self, parent: &str, name: &str, data: &[u8]) -> bool {
        if !Self::valid_name(name) {
            return false;
        }
        let Some(p) = self.node_mut(parent) else {
            return false;
        };
        if p.kind != NodeKind::Directory {
            return false;
        }
        if p.children.iter().any(|n| n.name == name) {
            return false;
        }
        p.children.push(Node::file(name, data));
        true
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        let n = self.node(path)?;
        if n.kind != NodeKind::File {
            return None;
        }
        Some(n.data.clone())
    }

    pub fn write(&mut self, path: &str, data: &[u8]) -> bool {
        let Some(n) = self.node_mut(path) else {
            return false;
        };
        if n.kind != NodeKind::File {
            return false;
        }
        n.data.clear();
        n.data.extend_from_slice(data);
        true
    }

    pub fn remove(&mut self, path: &str) -> bool {
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return false;
        }
        let Some(pos) = trimmed.rfind('/') else {
            return false;
        };
        let parent_path = if pos == 0 { "/" } else { &trimmed[..pos] };
        let name = &trimmed[pos + 1..];
        let Some(p) = self.node_mut(parent_path) else {
            return false;
        };
        let Some(i) = p.children.iter().position(|n| n.name == name) else {
            return false;
        };
        p.children.remove(i);
        true
    }

    pub fn rename(&mut self, path: &str, new_name: &str) -> bool {
        if !Self::valid_name(new_name) {
            return false;
        }
        let trimmed = path.trim_end_matches('/');
        if trimmed.is_empty() {
            return false;
        }
        let Some(pos) = trimmed.rfind('/') else {
            return false;
        };
        let parent_path = if pos == 0 { "/" } else { &trimmed[..pos] };
        let old_name = &trimmed[pos + 1..];

        let Some(p) = self.node_mut(parent_path) else {
            return false;
        };
        // Проверка на дубликат
        if p.children
            .iter()
            .any(|n| n.name == new_name && n.name != old_name)
        {
            return false;
        }
        let Some(i) = p.children.iter().position(|n| n.name == old_name) else {
            return false;
        };
        p.children[i].name = new_name.to_string();
        true
    }

    fn valid_name(name: &str) -> bool {
        !name.is_empty() && !name.contains('/') && name != "." && name != ".."
    }
}

// ---------- Helpers для работы с путями ----------

pub fn parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_string();
    }
    match trimmed.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => trimmed[..i].to_string(),
        None => "/".to_string(),
    }
}

pub fn join(parent: &str, name: &str) -> String {
    if parent == "/" || parent.is_empty() {
        alloc::format!("/{}", name)
    } else {
        alloc::format!("{}/{}", parent.trim_end_matches('/'), name)
    }
}