#[link(name = "modbus.dll", kind="dylib")]
extern {
    fn add(left: usize, right: usize) -> usize;
}

fn main() {
	unsafe {
        println!("2+2={}", add(2,2));
    }
}
