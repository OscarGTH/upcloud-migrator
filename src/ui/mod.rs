pub mod chat;
pub mod diff;
pub mod filebrowser;
pub mod generator;
pub mod pricing;
pub mod resources;
pub mod scanner;
pub mod splash;
pub mod theme;
pub mod todo;

use crate::app::{App, View};
use ratatui::Frame;

pub fn render(f: &mut Frame, app: &App) {
    match app.view {
        View::Splash => splash::render(f, app),
        View::FileBrowser => filebrowser::render(f, app),
        View::Scanner => scanner::render(f, app),
        View::Resources => resources::render(f, app),
        View::Generator => generator::render(f, app),
        View::DiffReview => diff::render(f, app),
        View::TodoReview => todo::render(f, app),
        View::Chat => chat::render(f, app),
        View::Pricing => pricing::render(f, app),
    }
}
