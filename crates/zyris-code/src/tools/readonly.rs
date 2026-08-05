//! Exposes `file_io` **read-only**.
//!
//! Handing out capkit's `LocalFileIo` as-is drags `write`·`remove`·`mkdir` along too. If there are
//! two ways to change a file, the agent picks a full overwrite and the diff spreads over the whole
//! file, and the approval gate has to be in two places. So the descriptor's tool list is filtered
//! before announcing — legal, since protocol §5 pins down "consumers discover tools by descriptor".
//!
//! **This node has no file-deleting tool at all.** Deleting is done by a human.

use std::path::PathBuf;

use async_trait::async_trait;
// The `serve` module itself is private. Its items are re-exported at the crate root, so use those.
use zyris::{CapabilityDescriptor, IncomingCall, Outgoing, Result, ServeCapability};
use zyris_capkit::LocalFileIo;
use zyris_caps::FileIoServer;

/// The four that are exposed. The rest are filtered out.
const READ_ONLY: &[&str] = &["stat", "list", "read", "read_stream"];

pub struct ReadOnlyFileIo(FileIoServer<LocalFileIo>);

impl ReadOnlyFileIo {
    pub fn new(root: PathBuf) -> ReadOnlyFileIo {
        ReadOnlyFileIo(FileIoServer(LocalFileIo::rooted(root)))
    }
}

#[async_trait]
impl ServeCapability for ReadOnlyFileIo {
    fn descriptor(&self) -> CapabilityDescriptor {
        let mut d = self.0.descriptor();
        d.tools.retain(|t| READ_ONLY.contains(&t.name.as_str()));
        d
    }

    async fn dispatch(&self, call: IncomingCall) -> Result<Outgoing> {
        // **A tool that wasn't announced must not be callable either.** Filtering only the list while
        // leaving dispatch open lets anyone who knows the name just call it — the filtering is moot.
        if !READ_ONLY.contains(&call.tool.as_str()) {
            return Err(zyris::unknown_tool("file_io", &call.tool));
        }
        self.0.dispatch(call).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With two write paths, the agent picks a full overwrite and the diff spreads over the whole file.
    #[test]
    fn the_announced_file_io_cannot_write() {
        let cap = ReadOnlyFileIo::new(PathBuf::from("/tmp")).descriptor();
        let names: Vec<&str> = cap.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read"), "읽기는 있어야 한다: {names:?}");
        for banned in ["write", "remove", "mkdir"] {
            assert!(!names.contains(&banned), "{banned}이 내줘지면 안 된다: {names:?}");
        }
    }

    /// It attaches only when the name and version are the values zyris sets — matching is on the (name, version) pair.
    #[test]
    fn it_still_announces_itself_as_file_io() {
        let cap = ReadOnlyFileIo::new(PathBuf::from("/tmp")).descriptor();
        assert_eq!(cap.name, "file_io");
        assert_eq!(cap.version, zyris_caps::file_io_capability().version);
    }

    /// A tool filtered from the list must be reported as missing even when called.
    #[tokio::test]
    async fn a_filtered_tool_cannot_be_called_anyway() {
        let cap = ReadOnlyFileIo::new(PathBuf::from("/tmp"));
        let call = IncomingCall {
            tool: "remove".into(),
            params: zyris::Payload::from_json(serde_json::json!({"path": "a"})),
            serialization: zyris::Serialization::Json,
        };
        assert!(cap.dispatch(call).await.is_err(), "거른 도구가 불려서는 안 된다");
    }
}
