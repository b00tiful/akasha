use crate::{queue::Queue, report::Report};

pub fn flush(queue: &mut Queue, mut send: impl FnMut(&str) -> bool) -> Report {
    let mut report = Report { sent: 0, failed: false, remaining: queue.len() };
    while let Some(id) = queue.front() {
        if !send(id) { report.failed = true; break; }
        queue.acknowledge();
        report.sent += 1;
    }
    report.remaining = queue.len();
    report
}
