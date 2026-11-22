//! Incremental view of a request body for the streaming request pipeline.
//!
//! [`Lexer`] turns arbitrary byte chunks of one JSON document into [`Event`]s
//! with absolute byte [`Span`]s and the [`Path`] of the value they belong to.
//! It holds no document content: memory is bounded by the nesting limit, one
//! key, one scalar and one escape sequence, whatever the body size.
//!
//! [`MetadataSniffer`] consumes those events and collects the handful of
//! top-level fields the proxy needs before or while a body streams (`model`,
//! `stream`, counts, …), independent of key order.
//!
//! # Two cursors over the same bytes
//!
//! Lexing is deterministic: two lexers fed the same bytes report the same
//! events with the same spans, however differently the bytes are chunked
//! (only the cut points between consecutive [`Event::StringChunk`]s move). A
//! "head" lexer can therefore annotate offsets while a "tail" lexer, running
//! behind it over the held bytes, rewrites selected string contents and copies
//! every other span byte-for-byte. Bytes between two consecutive spans are
//! only JSON whitespace, `,` and `:`.
//!
//! The response side has its own adapter (`json_stream::JsonStream`), which
//! restores placeholders while it forwards text and is tied to that job. This
//! module shares its conventions (decoded text paired with the raw spelling,
//! at most one incomplete UTF-8 sequence carried between chunks, a definite
//! invalid sequence ends inspection) but reports offsets instead of rewriting.

mod lexer;
mod sniffer;

#[cfg(test)]
mod tests;

pub use lexer::{Event, LexError, Lexer, LexerLimits, Path, Segment, Span, Step};
pub use sniffer::{sniff, MetadataSniffer, RequestMetadata, Shape, Sniffed, DEFAULT_FIELD_LIMIT};
