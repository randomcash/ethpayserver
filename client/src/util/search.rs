//! The list-search term: how it is normalised, and how it is debounced before
//! it reaches the server.
//!
//! Search used to be a client-side filter over the fetched page, so typing was
//! free. It is a query parameter now (RCS-231) — every distinct term is a
//! request — which is what the debounce below is for.

use std::cell::RefCell;
use std::rc::Rc;

use gloo_timers::callback::Timeout;
use leptos::prelude::*;
use send_wrapper::SendWrapper;

/// How long the search box must sit idle before its term is queried, in
/// milliseconds.
///
/// 300ms, chosen against typing speed rather than taste: an unhurried typist
/// leaves 150-250ms between keystrokes, so 300 collapses a whole word into one
/// request instead of one per character, while staying under the ~400ms where
/// the pause stops reading as "still typing" and starts reading as a slow app.
pub const SEARCH_DEBOUNCE_MS: u32 = 300;

/// The term to send as `search`, or `None` when the box holds nothing usable.
///
/// The server trims and treats blank as no filter, so this is not about making
/// it agree — it is about not spending a request on an edit that cannot change
/// the answer, and about `search=` never appearing in the URL empty.
#[must_use]
pub fn normalize_search(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The search term from `raw`, normalised and settled: it changes only once
/// typing has paused for [`SEARCH_DEBOUNCE_MS`].
///
/// The pending timer lives in an `Rc` this hook owns so that a new keystroke
/// *replaces* it — dropping a `Timeout` cancels it — and so `on_cleanup` can
/// cancel one still in flight. A timer that outlives its owner and writes a
/// disposed signal panics the whole app, which was RCS-220; here that is
/// navigating away mid-word.
pub fn use_debounced_search(raw: Signal<String>) -> Signal<Option<String>> {
    let (term, set_term) = signal(normalize_search(&raw.get_untracked()));

    let pending: Rc<RefCell<Option<Timeout>>> = Rc::new(RefCell::new(None));
    let pending_for_effect = pending.clone();
    Effect::new(move |_| {
        let next = normalize_search(&raw.get());
        if next == term.get_untracked() {
            // Nothing to query: the first run, and any edit that normalises
            // back to the term already on screen — trailing spaces, or typing
            // a character and deleting it again. Cancel rather than return,
            // or that round trip leaves the earlier timer armed and re-queries
            // a term the user has already backed out of.
            *pending_for_effect.borrow_mut() = None;
            return;
        }
        *pending_for_effect.borrow_mut() = Some(Timeout::new(SEARCH_DEBOUNCE_MS, move || {
            set_term.set(next);
        }));
    });

    let pending_for_cleanup = SendWrapper::new(pending);
    on_cleanup(move || {
        pending_for_cleanup.borrow_mut().take();
    });

    term.into()
}

#[cfg(test)]
mod tests {
    use super::normalize_search;

    #[test]
    fn an_empty_box_is_no_filter() {
        assert_eq!(normalize_search(""), None);
        assert_eq!(normalize_search("   "), None);
        assert_eq!(normalize_search("\t\n"), None);
    }

    #[test]
    fn a_term_is_trimmed_but_otherwise_untouched() {
        assert_eq!(normalize_search("  0xAbC "), Some("0xAbC".to_string()));
        // Case is the server's business — it matches case-insensitively — and
        // lowercasing here would only make the box disagree with the URL.
        assert_eq!(normalize_search("USD"), Some("USD".to_string()));
        // Inner whitespace is part of the term, not padding to strip.
        assert_eq!(
            normalize_search(" acme order "),
            Some("acme order".to_string())
        );
    }
}
