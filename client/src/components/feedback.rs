//! Reusable loading and error feedback components.

use leptos::prelude::*;

/// Full-page loading spinner with optional message.
#[component]
pub fn LoadingState(#[prop(default = "Loading...")] message: &'static str) -> impl IntoView {
    view! {
        <div class="loading-container">
            <div class="loading-spinner"></div>
            <p class="loading-text">{message}</p>
        </div>
    }
}

/// Inline loading indicator (for use inside cards/sections).
#[component]
pub fn LoadingInline(#[prop(default = "Loading...")] message: &'static str) -> impl IntoView {
    view! {
        <div class="loading-inline">
            <div class="loading-spinner-sm"></div>
            <span class="loading-text-sm">{message}</span>
        </div>
    }
}

/// Error display with retry button.
#[component]
pub fn ErrorState(
    #[prop(into)] message: String,
    #[prop(optional)] on_retry: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <div class="error-container">
            <div class="error-icon">
                <svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10"></circle>
                    <line x1="12" y1="8" x2="12" y2="12"></line>
                    <line x1="12" y1="16" x2="12.01" y2="16"></line>
                </svg>
            </div>
            <p class="error-message">{message}</p>
            {on_retry.map(|retry| view! {
                <button
                    class="btn btn-secondary btn-sm"
                    on:click=move |_| retry.run(())
                >
                    "Try again"
                </button>
            })}
        </div>
    }
}

/// Empty state placeholder.
#[component]
pub fn EmptyState(#[prop(into)] title: String, #[prop(into)] description: String) -> impl IntoView {
    view! {
        <div class="empty-state">
            <div class="empty-state-icon">
                <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="2" y="7" width="20" height="14" rx="2" ry="2"></rect>
                    <path d="M16 21V5a2 2 0 0 0-2-2h-4a2 2 0 0 0-2 2v16"></path>
                </svg>
            </div>
            <h3 class="empty-state-title">{title}</h3>
            <p class="empty-state-description">{description}</p>
        </div>
    }
}
