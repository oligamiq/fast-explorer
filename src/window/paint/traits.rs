use raqote::DrawTarget;

pub trait WindowPainter {
    fn paint(&self, dt: &mut DrawTarget);
}
