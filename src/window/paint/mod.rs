use raqote::DrawTarget;

use crate::State;

use self::{traits::WindowPainter, util::get_accent_color};
use raqote::*;

pub mod traits;
pub mod util;

impl WindowPainter for State {
    fn paint(&self, dt: &mut DrawTarget) {
        let width = dt.width();
        let height = dt.height();

        let color = get_accent_color();

        let mut pb = PathBuilder::new();
        pb.move_to(0., 0.);
        pb.line_to(width as f32, height as f32);
        pb.line_to(width as f32, 0.);
        pb.close();
        let path = pb.finish();

        dt.fill(
            &path,
            &Source::Solid(color.into()),
            &DrawOptions::new(),
        );
    }
}
