//! Recycled stream buffers for quiche (meow-openwrt).
//!
//! quiche copies every received STREAM frame, and every 4 KiB of data the
//! driver writes, into a buffer from its `BufFactory`. The default one
//! allocates an `Arc<[u8]>` each time: one malloc and one free per packet,
//! which musl's allocator makes a visible share of a router's hy2 CPU
//! (mimalloc bought +8-15 % for +20 MB RSS). This factory hands out blocks
//! from a small per-thread free list instead and takes them back when
//! quiche drops the last handle.
//!
//! Two size classes keep memory honest: received frames (at most one UDP
//! payload) sit in 1.5 KiB blocks, so a full receive window costs what it
//! did before, and the send path's 4 KiB chunks in 4 KiB blocks. Each
//! thread keeps at most [`KEEP_SMALL`] + [`KEEP_LARGE`] idle blocks
//! (~320 KiB); larger buffers are allocated as before and never pooled.

use std::cell::RefCell;
use std::sync::Arc;

/// Received STREAM frames: one QUIC packet's payload at most (1452 with
/// DPLPMTUD), rounded up.
const SMALL: usize = 1536;
/// quiche's `SEND_BUFFER_SIZE`: stream writes are split into 4 KiB chunks.
const LARGE: usize = 4096;
/// Idle blocks a thread keeps per class.
const KEEP_SMALL: usize = 128;
const KEEP_LARGE: usize = 32;

type Block = Arc<Vec<u8>>;

thread_local! {
    static FREE: RefCell<(Vec<Block>, Vec<Block>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// The quiche buffer factory of the hysteria2 connection.
#[derive(Debug, Clone, Default)]
pub(crate) struct PoolFactory;

/// A quiche stream buffer; its block goes back to the free list when the
/// last clone drops.
#[derive(Debug, Clone, Default)]
pub(crate) struct PoolBuf(Option<Block>);

impl AsRef<[u8]> for PoolBuf {
    fn as_ref(&self) -> &[u8] {
        self.0.as_deref().map_or(&[], Vec::as_slice)
    }
}

impl Drop for PoolBuf {
    fn drop(&mut self) {
        let Some(block) = self.0.take() else {
            return;
        };
        // Only the last handle recycles; a clone dropped elsewhere at the
        // same moment just lets the block be freed.
        if Arc::strong_count(&block) != 1 {
            return;
        }
        let _ = FREE.try_with(|free| {
            let mut free = free.borrow_mut();
            let (list, keep) = match block.capacity() {
                SMALL => (&mut free.0, KEEP_SMALL),
                LARGE => (&mut free.1, KEEP_LARGE),
                _ => return,
            };
            if list.len() < keep {
                list.push(block);
            }
        });
    }
}

impl quiche::BufFactory for PoolFactory {
    type Buf = PoolBuf;
    type DgramBuf = Vec<u8>;

    fn buf_from_slice(buf: &[u8]) -> PoolBuf {
        let class = match buf.len() {
            0 => return PoolBuf(None),
            n if n <= SMALL => SMALL,
            n if n <= LARGE => LARGE,
            _ => return PoolBuf(Some(Arc::new(buf.to_vec()))),
        };
        let recycled = FREE
            .try_with(|free| {
                let mut free = free.borrow_mut();
                if class == SMALL {
                    free.0.pop()
                } else {
                    free.1.pop()
                }
            })
            .ok()
            .flatten();
        let mut block = recycled.unwrap_or_else(|| Arc::new(Vec::with_capacity(class)));
        // Pooled blocks have no other handle (pushed only at count 1).
        let Some(data) = Arc::get_mut(&mut block) else {
            return PoolBuf(Some(Arc::new(buf.to_vec())));
        };
        data.clear();
        data.extend_from_slice(buf);
        PoolBuf(Some(block))
    }

    fn dgram_buf_from_slice(buf: &[u8]) -> Vec<u8> {
        buf.to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quiche::BufFactory;

    fn idle() -> (usize, usize) {
        FREE.with(|f| {
            let f = f.borrow();
            (f.0.len(), f.1.len())
        })
    }

    #[test]
    fn blocks_are_recycled_by_size_class() {
        let start = idle();
        let a = PoolFactory::buf_from_slice(&[1; 1200]);
        let b = PoolFactory::buf_from_slice(&[2; 4096]);
        assert_eq!(a.as_ref(), &[1; 1200][..]);
        assert_eq!(b.as_ref(), &[2; 4096][..]);
        let a2 = a.clone();
        drop(a);
        assert_eq!(
            idle(),
            start,
            "a live clone keeps the block out of the pool"
        );
        drop(a2);
        drop(b);
        let after = idle();
        assert_eq!(after, (start.0 + 1, start.1 + 1));
        // The recycled block is reused and its old contents are gone.
        let c = PoolFactory::buf_from_slice(&[3; 10]);
        assert_eq!(c.as_ref(), &[3; 10][..]);
        assert_eq!(idle(), (after.0 - 1, after.1));
    }

    #[test]
    fn oversized_and_empty_buffers_bypass_the_pool() {
        let start = idle();
        let big = PoolFactory::buf_from_slice(&[9; LARGE + 1]);
        assert_eq!(big.as_ref().len(), LARGE + 1);
        drop(big);
        drop(PoolFactory::buf_from_slice(&[]));
        assert_eq!(idle(), start);
    }
}
