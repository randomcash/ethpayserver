//! Reusable pagination component with page numbers, ellipsis, and navigation.

use leptos::prelude::*;

/// Page size used across list pages.
pub const PAGE_SIZE: i64 = 20;

/// Generate the list of page numbers to display, collapsing with `None` for
/// ellipsis when there are more than 7 pages.
///
/// Returns `Vec<Option<i64>>` where `Some(n)` is a page number (1-based) and
/// `None` represents an ellipsis separator.
fn page_numbers(current: i64, total_pages: i64) -> Vec<Option<i64>> {
    if total_pages <= 7 {
        return (1..=total_pages).map(Some).collect();
    }

    let mut pages: Vec<Option<i64>> = Vec::new();

    // Always show page 1
    pages.push(Some(1));

    if current > 3 {
        pages.push(None); // ellipsis
    }

    // Window around current page (at least 2 inner pages at boundaries)
    let start = (current - 1).max(2);
    let end = (current + 1).min(total_pages - 1).max(start + 1).min(total_pages - 1);
    for p in start..=end {
        pages.push(Some(p));
    }

    if current < total_pages - 2 {
        pages.push(None); // ellipsis
    }

    // Always show last page
    pages.push(Some(total_pages));

    pages
}

/// Reusable pagination component.
///
/// # Props
/// - `total`: Total number of items.
/// - `page_size`: Items per page.
/// - `current_offset`: Current 0-based offset into the result set.
/// - `on_page_change`: Callback receiving the new 0-based offset when a page
///   is selected.
/// - `item_label`: Label for items (e.g., "invoices", "payments").
#[component]
pub fn Pagination(
    total: i64,
    page_size: i64,
    current_offset: i64,
    on_page_change: impl Fn(i64) + Copy + 'static,
    #[prop(default = "items")] item_label: &'static str,
) -> impl IntoView {
    let total_pages = ((total as f64) / (page_size as f64)).ceil() as i64;
    let current_page = (current_offset / page_size) + 1;
    let showing_start = current_offset + 1;
    let showing_end = (current_offset + page_size).min(total);

    let pages = page_numbers(current_page, total_pages);

    view! {
        <div class="pagination">
            <div class="pagination-info">
                "Showing "<strong>{showing_start}"-"{showing_end}</strong>
                " of "<strong>{total}</strong>" "{item_label}
            </div>
            <div class="pagination-nav">
                // First
                <button
                    class="pagination-btn"
                    disabled=current_page <= 1
                    title="First page"
                    on:click=move |_| on_page_change(0)
                >
                    {view! { <IconFirst /> }}
                </button>
                // Previous
                <button
                    class="pagination-btn"
                    disabled=current_page <= 1
                    title="Previous page"
                    on:click=move |_| on_page_change(((current_page - 2) * page_size).max(0))
                >
                    {view! { <IconPrev /> }}
                </button>

                // Page numbers
                {pages.into_iter().map(|p| {
                    match p {
                        Some(page) => {
                            let is_current = page == current_page;
                            let offset = (page - 1) * page_size;
                            view! {
                                <button
                                    class=if is_current { "pagination-btn pagination-btn-active" } else { "pagination-btn pagination-btn-page" }
                                    disabled=is_current
                                    on:click=move |_| on_page_change(offset)
                                >
                                    {page}
                                </button>
                            }.into_any()
                        }
                        None => {
                            view! {
                                <span class="pagination-ellipsis">"..."</span>
                            }.into_any()
                        }
                    }
                }).collect_view()}

                // Next
                <button
                    class="pagination-btn"
                    disabled=current_page >= total_pages
                    title="Next page"
                    on:click=move |_| on_page_change(current_page * page_size)
                >
                    {view! { <IconNext /> }}
                </button>
                // Last
                <button
                    class="pagination-btn"
                    disabled=current_page >= total_pages
                    title="Last page"
                    on:click=move |_| on_page_change((total_pages - 1) * page_size)
                >
                    {view! { <IconLast /> }}
                </button>
            </div>
        </div>
    }
}

/// Double left chevron icon (First).
#[component]
fn IconFirst() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="11 17 6 12 11 7"></polyline>
            <polyline points="18 17 13 12 18 7"></polyline>
        </svg>
    }
}

/// Left chevron icon (Prev).
#[component]
fn IconPrev() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="15 18 9 12 15 6"></polyline>
        </svg>
    }
}

/// Right chevron icon (Next).
#[component]
fn IconNext() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="9 18 15 12 9 6"></polyline>
        </svg>
    }
}

/// Double right chevron icon (Last).
#[component]
fn IconLast() -> impl IntoView {
    view! {
        <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="13 17 18 12 13 7"></polyline>
            <polyline points="6 17 11 12 6 7"></polyline>
        </svg>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_numbers_single_page() {
        assert_eq!(page_numbers(1, 1), vec![Some(1)]);
    }

    #[test]
    fn page_numbers_two_pages() {
        assert_eq!(page_numbers(1, 2), vec![Some(1), Some(2)]);
        assert_eq!(page_numbers(2, 2), vec![Some(1), Some(2)]);
    }

    #[test]
    fn page_numbers_seven_pages_no_ellipsis() {
        let expected: Vec<Option<i64>> = (1..=7).map(Some).collect();
        assert_eq!(page_numbers(1, 7), expected);
        assert_eq!(page_numbers(4, 7), expected);
        assert_eq!(page_numbers(7, 7), expected);
    }

    #[test]
    fn page_numbers_many_pages_start() {
        // On page 1 of 20: 1 2 3 ... 20
        let result = page_numbers(1, 20);
        assert_eq!(result, vec![Some(1), Some(2), Some(3), None, Some(20),]);
    }

    #[test]
    fn page_numbers_many_pages_middle() {
        // On page 10 of 20: 1 ... 9 10 11 ... 20
        let result = page_numbers(10, 20);
        assert_eq!(
            result,
            vec![Some(1), None, Some(9), Some(10), Some(11), None, Some(20),]
        );
    }

    #[test]
    fn page_numbers_many_pages_end() {
        // On page 20 of 20: 1 ... 18 19 20
        let result = page_numbers(20, 20);
        assert_eq!(result, vec![Some(1), None, Some(19), Some(20),]);
    }

    #[test]
    fn page_numbers_near_start() {
        // On page 3 of 20: 1 2 3 4 ... 20
        let result = page_numbers(3, 20);
        assert_eq!(
            result,
            vec![Some(1), Some(2), Some(3), Some(4), None, Some(20),]
        );
    }

    #[test]
    fn page_numbers_near_end() {
        // On page 18 of 20: 1 ... 17 18 19 20
        let result = page_numbers(18, 20);
        assert_eq!(
            result,
            vec![Some(1), None, Some(17), Some(18), Some(19), Some(20),]
        );
    }
}
