use crate::{
    config::Config,
    model::{Analysis, Session},
};
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct Workspace {
    pub root: PathBuf,
}
fn safe(value: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-.".contains(&c))
        || value == "."
        || value == ".."
    {
        bail!("invalid storage identifier");
    }
    Ok(())
}
fn atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path.parent().context("missing parent")?;
    fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value)?;
    tmp.write_all(b"\n")?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
impl Workspace {
    pub fn init(root: &Path, config: &Config) -> Result<Self> {
        fs::create_dir_all(root)?;
        let ws = Self {
            root: root.canonicalize()?,
        };
        if !ws.data_dir().join("config.json").exists() {
            atomic(&ws.data_dir().join("config.json"), config)?;
        }
        Ok(ws)
    }
    pub fn discover(explicit: Option<&Path>) -> Result<Self> {
        let start = explicit
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?);
        let start = start.canonicalize().context("workspace path not found")?;
        if explicit.is_some() {
            if start.join(".jta/config.json").exists() {
                return Ok(Self { root: start });
            }
            bail!("no workspace at supplied path; run jta init");
        }
        for root in start.ancestors() {
            if root.join(".jta/config.json").exists() {
                return Ok(Self {
                    root: root.to_owned(),
                });
            }
            if root.join(".git").exists() {
                break;
            }
        }
        bail!("no .jta workspace found within this project; run jta init")
    }
    pub fn data_dir(&self) -> PathBuf {
        self.root.join(".jta")
    }
    pub fn config(&self) -> Result<Config> {
        Ok(serde_json::from_slice(&fs::read(
            self.data_dir().join("config.json"),
        )?)?)
    }
    pub fn save_session(&self, session: &Session) -> Result<()> {
        safe(&session.id)?;
        safe(&session.revision)?;
        // Store immutable evidence first; publish the latest pointer only after persistence.
        self.save_json(
            "revisions",
            &format!("{}--{}", session.id, session.revision),
            session,
        )?;
        self.save_json("sessions", &session.id, session)?;
        // Parser migrations may namespace old IDs. Retire only current pointers;
        // immutable revisions and analyses remain available to runs and snapshots.
        let version = session.parser_version.parse::<u32>().unwrap_or(0);
        for previous in self.sessions()? {
            if previous.id != session.id
                && previous.source_path == session.source_path
                && previous.agent == session.agent
                && previous.parser_version.parse::<u32>().unwrap_or(0) < version
            {
                safe(&previous.id)?;
                fs::remove_file(
                    self.data_dir()
                        .join("sessions")
                        .join(format!("{}.json", previous.id)),
                )?;
            }
        }
        Ok(())
    }
    pub fn sessions(&self) -> Result<Vec<Session>> {
        self.list_json("sessions")
    }
    pub fn session(&self, id: &str, revision: Option<&str>) -> Result<Session> {
        safe(id)?;
        match revision {
            Some(r) => {
                safe(r)?;
                self.load_json("revisions", &format!("{id}--{r}"))
            }
            None => self.load_json("sessions", id),
        }
    }
    pub fn save_analysis(&self, a: &Analysis) -> Result<()> {
        self.save_json("analyses", &a.id, a)
    }
    pub fn analyses(&self) -> Result<Vec<Analysis>> {
        self.list_json("analyses")
    }
    pub fn save_json<T: Serialize>(&self, collection: &str, id: &str, value: &T) -> Result<()> {
        safe(collection)?;
        safe(id)?;
        atomic(
            &self.data_dir().join(collection).join(format!("{id}.json")),
            value,
        )
    }
    pub fn load_json<T: DeserializeOwned>(&self, collection: &str, id: &str) -> Result<T> {
        safe(collection)?;
        safe(id)?;
        let p = self.data_dir().join(collection).join(format!("{id}.json"));
        Ok(serde_json::from_slice(&fs::read(&p).with_context(
            || format!("cannot load {}", p.display()),
        )?)?)
    }
    pub fn list_json<T: DeserializeOwned>(&self, collection: &str) -> Result<Vec<T>> {
        safe(collection)?;
        let dir = self.data_dir().join(collection);
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut paths = fs::read_dir(dir)?
            .map(|e| e.map(|e| e.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        paths.sort();
        paths
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "json"))
            .map(|p| {
                serde_json::from_slice(&fs::read(&p)?)
                    .with_context(|| format!("corrupt artifact {}", p.display()))
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn revision_history_and_paths() {
        let d = tempfile::tempdir().unwrap();
        let w = Workspace::init(d.path(), &Config::default()).unwrap();
        let mut s = Session {
            id: "s_1".into(),
            revision: "a".into(),
            ..Default::default()
        };
        w.save_session(&s).unwrap();
        s.revision = "b".into();
        w.save_session(&s).unwrap();
        assert_eq!(w.sessions().unwrap().len(), 1);
        assert_eq!(w.session("s_1", Some("a")).unwrap().revision, "a");
        assert!(w.save_json("../bad", "x", &s).is_err());
    }
}
