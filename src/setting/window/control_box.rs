#[derive(Debug, Clone, Copy)]
pub struct ControlBoxSetting {
    pub caption_wide: i32,
    pub caption_direction: CaptionDirection,
    pub box_width: i32,
    pub box_height: i32,
    pub maximize_button: bool,
    pub minimize_button: bool,
    pub close_button: bool,
    pub position_x: ControlBoxPositionAxis,
    pub position_y: ControlBoxPositionAxis,
}

#[derive(Debug, Clone, Copy)]
pub enum ControlBoxPositionAxis {
    First,
    Center { margin: i32 },
    Last,
}

impl Default for ControlBoxSetting {
    fn default() -> Self {
        Self {
            caption_wide: 30,
            caption_direction: CaptionDirection::Top,
            box_width: 46,
            box_height: 30,
            maximize_button: true,
            minimize_button: true,
            close_button: true,
            position_x: ControlBoxPositionAxis::Last,
            position_y: ControlBoxPositionAxis::Center { margin: 0 },
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum CaptionDirection {
    Left,
    Right,
    Top,
    Bottom,
}
