fn main() -> Result<(), open365_ui_host::HostError> {
    open365_preview::run(open365_preview::AppKind::Sheets)
}
