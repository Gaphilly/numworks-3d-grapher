use crate::camera::{Camera, ScreenPoint};
use crate::eadk::{self, Color, Rect};
use crate::surface::{self, COLUMNS, ROWS};

const SCREEN_WIDTH: usize = 320;
const SCREEN_HEIGHT: usize = 240;
const BAND_HEIGHT: usize = 8;
const BACKGROUND: Color = Color { rgb565: 0xffff };
const WIRE: Color = Color { rgb565: 0x001f };

pub fn render(camera: &Camera) {
    let mut projected = [[ScreenPoint::INVALID; COLUMNS]; ROWS];
    let projector = camera.projector();
    let mut row = 0;
    while row < ROWS {
        let mut column = 0;
        while column < COLUMNS {
            projected[row][column] = projector.project(surface::point(column, row));
            column += 1;
        }
        row += 1;
    }

    eadk::display::wait_for_vblank();
    let mut pixels = [BACKGROUND; SCREEN_WIDTH * BAND_HEIGHT];
    let mut band_y = 0;
    while band_y < SCREEN_HEIGHT {
        pixels.fill(BACKGROUND);

        row = 0;
        while row < ROWS {
            let mut column = 0;
            while column < COLUMNS {
                if column + 1 < COLUMNS {
                    draw_line(
                        &mut pixels,
                        band_y,
                        projected[row][column],
                        projected[row][column + 1],
                    );
                }
                if row + 1 < ROWS {
                    draw_line(
                        &mut pixels,
                        band_y,
                        projected[row][column],
                        projected[row + 1][column],
                    );
                }
                column += 1;
            }
            row += 1;
        }

        eadk::display::push_rect(
            Rect {
                x: 0,
                y: band_y as u16,
                width: SCREEN_WIDTH as u16,
                height: BAND_HEIGHT as u16,
            },
            &pixels,
        );
        band_y += BAND_HEIGHT;
    }
}

fn draw_line(pixels: &mut [Color], band_y: usize, start: ScreenPoint, end: ScreenPoint) {
    if !start.is_visible() || !end.is_visible() {
        return;
    }

    let band_bottom = band_y as i32 + BAND_HEIGHT as i32 - 1;
    let min_y = if start.y < end.y { start.y } else { end.y } as i32;
    let max_y = if start.y > end.y { start.y } else { end.y } as i32;
    if max_y < band_y as i32 || min_y > band_bottom {
        return;
    }

    let mut x0 = start.x as i32;
    let mut y0 = start.y as i32;
    let x1 = end.x as i32;
    let y1 = end.y as i32;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;

    loop {
        if x0 >= 0 && x0 < SCREEN_WIDTH as i32 && y0 >= band_y as i32 && y0 <= band_bottom {
            let local_y = y0 as usize - band_y;
            let index = local_y * SCREEN_WIDTH + x0 as usize;
            if index < pixels.len() {
                pixels[index] = WIRE;
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let twice_error = 2 * error;
        if twice_error >= dy {
            error += dy;
            x0 += sx;
        }
        if twice_error <= dx {
            error += dx;
            y0 += sy;
        }
    }
}
