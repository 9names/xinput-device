use crate::xinput::ControllerData;

/// xbox 360 controller inputs
pub struct XboxGamepad {
    pub dpad_up: bool,
    pub dpad_down: bool,
    pub dpad_left: bool,
    pub dpad_right: bool,
    pub btn_start: bool,
    pub btn_back: bool,
    pub btn_left_thumb: bool,
    pub btn_right_thumb: bool,
    pub btn_left_shoulder: bool,
    pub btn_right_shoulder: bool,
    pub btn_guide: bool,
    pub btn_a: bool,
    pub btn_b: bool,
    pub btn_x: bool,
    pub btn_y: bool,
    pub trigger_left: i8,
    pub trigger_right: i8,
    pub thumb_left_x: i16,
    pub thumb_left_y: i16,
    pub thumb_right_x: i16,
    pub thumb_right_y: i16,
}

impl From<XboxGamepad> for ControllerData {
    fn from(joy: XboxGamepad) -> Self {
        let mut xinput_data = [0_u8; 12];

        // little helper closure for mapping button to bit offset
        let map_button = |to_bit, button: bool| {
            if button {
                1_u8 << to_bit
            } else {
                0
            }
        };

        xinput_data[0] = map_button(0, joy.dpad_up)
            | map_button(1, joy.dpad_down)
            | map_button(2, joy.dpad_left)
            | map_button(3, joy.dpad_right)
            | map_button(4, joy.btn_start)
            | map_button(5, joy.btn_back)
            | map_button(6, joy.btn_left_thumb)
            | map_button(7, joy.btn_right_thumb);

        xinput_data[1] = map_button(0, joy.btn_left_shoulder)
            | map_button(1, joy.btn_right_shoulder)
            | map_button(2, joy.btn_guide)
            // bit 3 is unused
            | map_button(4, joy.btn_a)
            | map_button(5, joy.btn_b)
            | map_button(6, joy.btn_y)
            | map_button(7, joy.btn_x);

        [xinput_data[2]] = joy.trigger_left.to_le_bytes();
        [xinput_data[3]] = joy.trigger_right.to_le_bytes();

        [xinput_data[4], xinput_data[5]] = joy.thumb_left_x.to_le_bytes();
        [xinput_data[6], xinput_data[7]] = joy.thumb_left_y.to_le_bytes();
        [xinput_data[8], xinput_data[9]] = joy.thumb_right_x.to_le_bytes();
        [xinput_data[10], xinput_data[11]] = joy.thumb_right_y.to_le_bytes();

        Self(xinput_data)
    }
}
