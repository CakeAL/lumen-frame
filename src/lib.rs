pub mod generate;
pub mod helper;
pub mod params;
pub mod photo;
pub mod process;

#[derive(Debug, Clone, Copy)]
pub enum Position {
    Center,
    Up,
    Right,
    Bottom,
    Left,
}
