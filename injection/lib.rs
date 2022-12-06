//! Requires libinjection to be built

use std::ffi::{c_char, CStr};
use std::net;
use std::net::Ipv4Addr;
use std::time::Instant;

use chrono::Utc;

use udcp::metrics;

static mut TRACE_IP: Option<Ipv4Addr> = None;
static mut DATA: metrics::Data = metrics::Data::new();
static mut START: Option<Instant> = None;
static mut TOTAL_MSG: u32 = 0;
static mut TOTAL_DROPS: u32 = 0;

#[no_mangle]
pub extern "C" fn inject_start() {
    unsafe {
        let _ = START.insert(Instant::now());
    }
}

#[no_mangle]
pub extern "C" fn trace_server_ip(ip: *const c_char) {
    unsafe {
        let _ = TRACE_IP.insert(CStr::from_ptr(ip).to_str().unwrap().parse().unwrap());
    }
}

#[no_mangle]
pub extern "C" fn push_msg(seq: u32) {
    unsafe {
        DATA.timestamps_msg.push(Utc::now());
        DATA.msgs.push(seq);
        TOTAL_MSG += 1;
    }
}

#[no_mangle]
pub extern "C" fn push_drop(seq: u32) {
    unsafe {
        DATA.timestamps_drop.push(Utc::now());
        DATA.drops.push(seq);
        TOTAL_DROPS += 1;
    }
}

#[no_mangle]
pub extern "C" fn push_ack(seq: u32) {
    unsafe {
        DATA.timestamps_ack.push(Utc::now());
        DATA.acks.push(seq);
    }
}

#[no_mangle]
pub extern "C" fn end_total(total: u32) {
    unsafe {
        DATA.execution_time = START.unwrap().elapsed();
        DATA.throughput_mo = total as f32 / 1000000. / DATA.execution_time.as_secs_f32();

        println!("Finished execution !");
        println!();
        println!("Execution time : {} ms", DATA.execution_time.as_millis(),);
        println!("File size : {:.2} Mo", total as f32 / 1000000.);
        println!("Total throughput : {} Mo/s", DATA.throughput_mo);
        println!(
            "Theoretical drop rate : {:.2} % (from throughput)",
            DATA.throughput_mo / 5. + 1.
        );
        println!("Messages received : {}", TOTAL_MSG);
        println!("Messages dropped : {}", TOTAL_DROPS);
        println!(
            "Drop rate : {:.2} %",
            TOTAL_DROPS as f32 / TOTAL_MSG as f32 * 100.
        );
        println!();
        print!("Sending trace data to server ... ");

        let conn = net::TcpStream::connect((TRACE_IP.unwrap(), 5000)).unwrap();
        bincode::serialize_into(conn, &DATA).unwrap();
        println!("Done !");
    }
}
