//! 404 Not Found page.

use leptos::prelude::*;
use ui_kit::components::{Button, ButtonVariant, Card};

/// 404 Not Found page.
#[component]
pub fn NotFoundPage() -> impl IntoView {
    view! {
        <div class="evm-page evm-not-found">
            <Card>
                <div class="evm-not-found-content">
                    <h1 class="evm-not-found-code">"404"</h1>
                    <h2 class="evm-not-found-title">"Page Not Found"</h2>
                    <p class="evm-not-found-message">
                        "The page you're looking for doesn't exist or has been moved."
                    </p>
                    <a href="/evm">
                        <Button variant=ButtonVariant::Primary>
                            "Go to Dashboard"
                        </Button>
                    </a>
                </div>
            </Card>
        </div>
    }
}

/// Not found page styles.
#[allow(dead_code)]
pub const NOT_FOUND_STYLES: &str = r#"
.evm-not-found {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 60vh;
}

.evm-not-found-content {
    text-align: center;
    padding: var(--ps-spacing-xl);
}

.evm-not-found-code {
    font-size: 5rem;
    font-weight: 700;
    color: var(--ps-text-muted);
    margin: 0;
    line-height: 1;
}

.evm-not-found-title {
    font-size: 1.5rem;
    margin: var(--ps-spacing-md) 0;
}

.evm-not-found-message {
    color: var(--ps-text-muted);
    margin-bottom: var(--ps-spacing-lg);
}
"#;
