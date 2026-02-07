use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn enumerate_paths(root: &Path) -> Vec<PathBuf> {
    fn rec(dir: &Path, acc: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for ent in rd.flatten() {
            let p = ent.path();
            acc.push(p.clone());
            if ent.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                rec(&p, acc);
            }
        }
    }

    let mut acc = Vec::new();
    rec(root, &mut acc);
    acc
}

pub(crate) fn smartcase_name_contains(name: &str, pat: &str) -> bool {
    let insensitive = !pat.chars().any(|c| c.is_uppercase());
    if insensitive {
        name.to_lowercase().contains(&pat.to_lowercase())
    } else {
        name.contains(pat)
    }
}

pub(crate) fn smartcase_basename_contains(path: &Path, pat: &str) -> bool {
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
    smartcase_name_contains(name, pat)
}
