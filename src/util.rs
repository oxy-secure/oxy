crate fn format_throughput(bytes: u64, seconds: u64) -> String {
    let seconds = if seconds != 0 { seconds } else { 1 };
    let mut throughput = bytes / seconds;
    let mut throughput_decimal = (bytes * 10) / seconds;
    let mut unit = "B/s";

    if throughput > 1024 {
        throughput_decimal = (throughput * 10) / 1024;
        throughput /= 1024;
        unit = "KiB/s";
    }

    if throughput > 1024 {
        throughput_decimal = (throughput * 10) / 1024;
        throughput /= 1024;
        unit = "MiB/s";
    }

    if throughput > 1024 {
        throughput_decimal = (throughput * 10) / 1024;
        throughput /= 1024;
        unit = "GiB/s";
    }
    format!("{}.{} {}", throughput, throughput_decimal % 10, unit)
}

crate fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{} B", bytes);
    }
    let mut bytes_decimal = (bytes * 10) / 1024;
    let mut bytes = bytes / 1024;
    let mut unit = "KiB";

    if bytes > 1024 {
        bytes_decimal = (bytes * 10) / 1024;
        bytes /= 1024;
        unit = "MiB"
    }
    if bytes > 1024 {
        bytes_decimal = (bytes * 10) / 1024;
        bytes /= 1024;
        unit = "GiB"
    }
    if bytes > 1024 {
        bytes_decimal = (bytes * 10) / 1024;
        bytes /= 1024;
        unit = "TiB"
    }
    format!("{}.{} {}", bytes, bytes_decimal % 10, unit)
}
