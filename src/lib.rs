pub mod generate;
pub mod params;
pub mod photo;
pub mod process;

#[derive(Debug, Clone)]
pub enum Position {
    Center,
    Up,
    Right,
    Bottom,
    Left,
}