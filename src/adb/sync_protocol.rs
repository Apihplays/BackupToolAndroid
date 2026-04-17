/// Placeholder for direct ADB sync protocol implementation.
/// Currently we use adb CLI wrapper in client.rs which uses the sync protocol under the hood.
/// A future optimization would be to implement the sync protocol directly over TCP
/// to localhost:5037 for even lower overhead.
///
/// The sync protocol uses these commands:
/// - LIST: enumerate directory entries
/// - STAT: get file metadata (mode, size, mtime)
/// - RECV: download file in 64KB chunks (DATA packets)
/// - SEND: upload file in 64KB chunks
/// - DONE: signals end of transfer
///
/// All integers are little-endian, packets are 8 bytes (4-byte ID + 4-byte length).

pub struct SyncClient;

impl SyncClient {
    /// Future: direct protocol implementation
    pub fn new() -> Self {
        Self
    }
}
