// test_page.rs — diagnostic page (aurora://test) running auto-tests for the active engine
pub fn get_test_html() -> String {
    include_str!("test.html").to_string()
}
