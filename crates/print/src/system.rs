//! The hand-off to the operating system's print service.
//!
//! [`PrintSystem`] is what the Print page talks to. [`OsPrintSystem`] implements it with the
//! `printers` crate (winspool on Windows, CUPS elsewhere); [`MemoryPrintSystem`] keeps jobs in
//! memory for tests and for the agent control surface's dry run.
//!
//! The job is always a finished PDF. CUPS converts PDF itself and applies the IPP attributes
//! sent with the job. The Windows spooler takes the bytes as a RAW job: that prints only on a
//! queue whose driver reads PDF (see `docs/architecture/printing.md`), and the job attributes
//! (sides, media) are not sent. [`SubmitReport::ignored`] lists what was not sent so the page can
//! say so.

use std::fmt;

use printers::common::base::job::PrinterJobOptions;
use printers::common::base::printer::PrinterState as RawState;
use printers::common::converters::Converter;

use crate::settings::Sides;

/// What a queue is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterState {
    Ready,
    Printing,
    Paused,
    Offline,
    Unknown,
}

/// A printer as the printer list shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrinterInfo {
    /// The name the list shows.
    pub name: String,
    /// The name the system takes back ([`PrintSystem::submit`]).
    pub system_name: String,
    pub is_default: bool,
    pub state: PrinterState,
    pub location: String,
    /// The driver or make-and-model text.
    pub driver: String,
    /// Whether a PDF job is known to print on this queue. `false` means "not known to": a
    /// Windows queue whose driver is not PostScript or PDF.
    pub reads_pdf: bool,
}

/// Job attributes beyond the PDF itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobOptions {
    /// The job name the queue shows.
    pub name: String,
    pub sides: Sides,
    /// IPP `media` keyword of the paper the PDF is laid out for, when the paper is a known size.
    pub media: Option<&'static str>,
}

/// What a successful submission reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitReport {
    pub job_id: u64,
    /// Job attributes the system path did not send (`sides`, `media`).
    pub ignored: Vec<&'static str>,
}

/// Why a job was not sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitError {
    /// No printer is chosen.
    NoPrinter,
    /// The chosen printer is not in the system's list any more.
    UnknownPrinter(String),
    /// The system refused the job (its message).
    Failed(String),
}

impl fmt::Display for SubmitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPrinter => write!(f, "Choose a printer first."),
            Self::UnknownPrinter(n) => write!(f, "The printer \"{n}\" is not available."),
            Self::Failed(m) => write!(f, "The printer did not take the job: {m}"),
        }
    }
}

impl std::error::Error for SubmitError {}

/// The print service the Print page uses.
pub trait PrintSystem {
    /// The installed printers, default first.
    fn printers(&self) -> Vec<PrinterInfo>;

    /// Sends `pdf` to `printer` (a [`PrinterInfo::system_name`]).
    ///
    /// # Errors
    /// [`SubmitError`] when the printer is unknown or the system refuses the job.
    fn submit(
        &mut self,
        printer: &str,
        pdf: &[u8],
        options: &JobOptions,
    ) -> Result<SubmitReport, SubmitError>;
}

fn state_of(s: &RawState) -> PrinterState {
    match s {
        RawState::READY => PrinterState::Ready,
        RawState::OFFLINE => PrinterState::Offline,
        RawState::PAUSED => PrinterState::Paused,
        RawState::PRINTING => PrinterState::Printing,
        RawState::UNKNOWN => PrinterState::Unknown,
    }
}

/// Whether a driver or model text names a page description language a PDF job prints on.
fn driver_reads_pdf(driver: &str) -> bool {
    let d = driver.to_ascii_lowercase();
    d.contains("postscript")
        || d.contains(" ps")
        || (d.contains("pdf") && !d.contains("print to pdf"))
}

/// The operating system's print service (`printers`: winspool, CUPS).
#[derive(Debug, Default, Clone, Copy)]
pub struct OsPrintSystem;

impl PrintSystem for OsPrintSystem {
    fn printers(&self) -> Vec<PrinterInfo> {
        let mut list: Vec<PrinterInfo> = printers::get_printers()
            .into_iter()
            .map(|p| PrinterInfo {
                reads_pdf: !cfg!(windows) || driver_reads_pdf(&p.driver_name),
                state: state_of(&p.state),
                name: p.name,
                system_name: p.system_name,
                is_default: p.is_default,
                location: p.location,
                driver: p.driver_name,
            })
            .collect();
        list.sort_by_key(|p| !p.is_default);
        list
    }

    fn submit(
        &mut self,
        printer: &str,
        pdf: &[u8],
        options: &JobOptions,
    ) -> Result<SubmitReport, SubmitError> {
        let target = printers::get_printer_by_name(printer)
            .ok_or_else(|| SubmitError::UnknownPrinter(printer.to_owned()))?;
        let mut props: Vec<(&str, &str)> = Vec::new();
        let mut ignored = Vec::new();
        if cfg!(windows) {
            ignored.push("sides");
            if options.media.is_some() {
                ignored.push("media");
            }
        } else {
            props.push(("document-format", "application/pdf"));
            props.push(("sides", options.sides.ipp()));
            props.push(("print-scaling", "none"));
            if let Some(media) = options.media {
                props.push(("media", media));
            }
        }
        let job = PrinterJobOptions {
            name: Some(&options.name),
            raw_properties: &props,
            converter: Converter::None,
        };
        let job_id = target
            .print(pdf, job)
            .map_err(|e| SubmitError::Failed(e.message))?;
        Ok(SubmitReport { job_id, ignored })
    }
}

/// One job a [`MemoryPrintSystem`] took.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submitted {
    pub printer: String,
    pub pdf: Vec<u8>,
    pub options: JobOptions,
}

/// A print service that keeps the jobs it is given.
#[derive(Debug, Default, Clone)]
pub struct MemoryPrintSystem {
    pub installed: Vec<PrinterInfo>,
    pub jobs: Vec<Submitted>,
}

impl MemoryPrintSystem {
    /// A service with the named printers, the first being the default.
    #[must_use]
    pub fn with_printers(names: &[&str]) -> Self {
        let installed = names
            .iter()
            .enumerate()
            .map(|(i, n)| PrinterInfo {
                name: (*n).to_owned(),
                system_name: (*n).to_owned(),
                is_default: i == 0,
                state: PrinterState::Ready,
                location: String::new(),
                driver: "Test PostScript".to_owned(),
                reads_pdf: true,
            })
            .collect();
        Self {
            installed,
            jobs: Vec::new(),
        }
    }
}

impl PrintSystem for MemoryPrintSystem {
    fn printers(&self) -> Vec<PrinterInfo> {
        self.installed.clone()
    }

    fn submit(
        &mut self,
        printer: &str,
        pdf: &[u8],
        options: &JobOptions,
    ) -> Result<SubmitReport, SubmitError> {
        if !self.installed.iter().any(|p| p.system_name == printer) {
            return Err(SubmitError::UnknownPrinter(printer.to_owned()));
        }
        self.jobs.push(Submitted {
            printer: printer.to_owned(),
            pdf: pdf.to_vec(),
            options: options.clone(),
        });
        Ok(SubmitReport {
            job_id: self.jobs.len() as u64,
            ignored: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_names_decide_pdf_support() {
        assert!(driver_reads_pdf("HP Universal Printing PS"));
        assert!(driver_reads_pdf("Generic PostScript Printer"));
        assert!(!driver_reads_pdf("Microsoft Print To PDF"));
        assert!(!driver_reads_pdf("Canon LBP2900"));
    }

    #[test]
    fn memory_system_keeps_jobs_and_refuses_unknown_printers() {
        let mut sys = MemoryPrintSystem::with_printers(&["Office", "Lab"]);
        let opts = JobOptions {
            name: "a".to_owned(),
            sides: Sides::OneSided,
            media: None,
        };
        assert_eq!(sys.printers()[0].name, "Office");
        let r = sys.submit("Lab", b"%PDF", &opts).expect("submitted");
        assert_eq!(r.job_id, 1);
        assert_eq!(sys.jobs[0].printer, "Lab");
        assert_eq!(
            sys.submit("Nope", b"%PDF", &opts),
            Err(SubmitError::UnknownPrinter("Nope".to_owned()))
        );
    }
}
