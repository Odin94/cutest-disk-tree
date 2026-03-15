// If you ever get "STATUS_ENTRYPOINT_NOT_FOUND", it is likely because we do some weird stuff with the build in here
fn main() {
    tauri_build::build()
}
