use crate::cli::{Args, Subcommands};
use crate::transformd::TransformD;
use eframe::egui;
use libmonado as mnd;
use std::sync::mpsc;
use std::thread;

// A message sent from the worker thread to the GUI thread
enum WorkerMessage {
    Devices(anyhow::Result<Vec<(String, u32, TransformD, Vec<(String, String)>)>>),
    CalibrationFinished(anyhow::Result<()>),
}

#[derive(Default)]
struct DeviceList {
    // (Serial, Name)
    devices: Vec<(String, String)>,
}

impl DeviceList {
    fn from_worker_message(data: Vec<(String, u32, TransformD, Vec<(String, String)>)>) -> Self {
        let mut devices = vec![];
        for (_, _, _, dev_list) in data {
            devices.extend(dev_list);
        }
        Self { devices }
    }
}

struct MotocApp {
    worker_rx: mpsc::Receiver<WorkerMessage>,
    worker_tx: mpsc::Sender<WorkerMessage>,
    devices: DeviceList,
    selected_src: Option<String>,
    selected_dst: Option<String>,
    status_text: String,
    is_calibrating: bool,
}

impl Default for MotocApp {
    fn default() -> Self {
        let (worker_tx, worker_rx) = mpsc::channel();
        let app = Self {
            worker_rx,
            worker_tx,
            devices: DeviceList::default(),
            selected_src: None,
            selected_dst: None,
            status_text: "Click 'Refresh' to find devices.".to_string(),
            is_calibrating: false,
        };
        app.refresh_devices();
        app
    }
}

impl MotocApp {
    fn refresh_devices(&self) {
        let tx = self.worker_tx.clone();
        if self.is_calibrating {
            return;
        }
        thread::spawn(move || {
            let result = mnd::Monado::auto_connect().map_or_else(
                |e| Err(anyhow::anyhow!("Failed to connect to Monado: {}", e)),
                |monado| crate::get_device_list(&monado),
            );
            tx.send(WorkerMessage::Devices(result)).unwrap();
        });
    }

    fn start_calibration(&mut self, src: String, dst: String) {
        if self.is_calibrating {
            return;
        }
        self.is_calibrating = true;
        self.status_text = format!("Calibrating {} and {}...", src, dst);

        let tx = self.worker_tx.clone();
        thread::spawn(move || {
            let result = (|| -> anyhow::Result<()> {
                let monado = mnd::Monado::auto_connect().map_err(anyhow::Error::msg)?;
                let args = Args {
                    command: Subcommands::Calibrate {
                        src,
                        dst,
                        r#continue: false,
                        samples: Some(500),
                        profile: "last".to_string(),
                    },
                    wait: false,
                };
                // This is a blocking call, running in a separate thread
                crate::xr_loop(args, monado, Default::default())?;
                Ok(())
            })();
            tx.send(WorkerMessage::CalibrationFinished(result))
                .unwrap();
        });
    }
}

impl eframe::App for MotocApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check for messages from worker thread
        if let Ok(msg) = self.worker_rx.try_recv() {
            match msg {
                WorkerMessage::Devices(Ok(devices)) => {
                    self.devices = DeviceList::from_worker_message(devices);
                    if self.devices.devices.is_empty() {
                        self.status_text = "No devices found.".to_string();
                    } else {
                        self.status_text = format!("Found {} devices.", self.devices.devices.len());
                        // Auto-select devices based on common names
                        if self.selected_src.is_none() {
                            if let Some((serial, _)) = self.devices.devices.iter().find(|d| d.1.contains("HMD")) {
                                self.selected_src = Some(serial.clone());
                            }
                        }
                        if self.selected_dst.is_none() {
                            if let Some((serial, _)) = self.devices.devices.iter().find(|d| d.0.starts_with("LHR-")) {
                                self.selected_dst = Some(serial.clone());
                            }
                        }
                    }
                }
                WorkerMessage::Devices(Err(e)) => {
                    self.status_text = format!("Error finding devices: {}", e);
                }
                WorkerMessage::CalibrationFinished(Ok(())) => {
                    self.status_text = "Calibration successful!".to_string();
                    self.is_calibrating = false;
                }
                WorkerMessage::CalibrationFinished(Err(e)) => {
                    self.status_text = format!("Calibration failed: {}", e);
                    self.is_calibrating = false;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Motoc Calibrator");
            ui.separator();

            ui.horizontal(|ui| {
                ui.add_enabled(!self.is_calibrating, egui::Label::new("Status:"));
                ui.label(&self.status_text);
                if ui.add_enabled(!self.is_calibrating, egui::Button::new("Refresh")).clicked() {
                    self.status_text = "Refreshing...".to_string();
                    self.refresh_devices();
                }
            });

            ui.separator();

            let src_devices = self.devices.devices.clone();
            let dst_devices = self.devices.devices.clone();

            let src_text = self.selected_src.as_ref().map_or("Select Source Device".to_string(), |s| s.clone());
            ui.add_enabled_ui(!self.is_calibrating, |ui| {
                egui::ComboBox::from_label("Source Device (e.g., HMD)")
                    .selected_text(src_text)
                    .show_ui(ui, |ui| {
                        for (serial, name) in src_devices {
                            ui.selectable_value(&mut self.selected_src, Some(serial.clone()), format!("{} ({})", name, serial));
                        }
                    });
            });

            let dst_text = self.selected_dst.as_ref().map_or("Select Destination Device".to_string(), |s| s.clone());
            ui.add_enabled_ui(!self.is_calibrating, |ui| {
                egui::ComboBox::from_label("Destination Device (e.g., Tracker)")
                    .selected_text(dst_text)
                    .show_ui(ui, |ui| {
                        for (serial, name) in dst_devices {
                            ui.selectable_value(&mut self.selected_dst, Some(serial.clone()), format!("{} ({})", name, serial));
                        }
                    });
            });

            ui.separator();

            if ui.add_enabled(!self.is_calibrating, egui::Button::new("Start Calibration")).clicked() {
                if let (Some(src), Some(dst)) = (self.selected_src.clone(), self.selected_dst.clone()) {
                    self.start_calibration(src, dst);
                } else {
                    self.status_text = "Please select both a source and a destination device.".to_string();
                }
            }
        });
    }
}

use eframe::egui::ViewportBuilder;

pub fn run_gui() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default().with_inner_size(egui::vec2(500.0, 250.0)),
        ..Default::default()
    };
    eframe::run_native(
        "Motoc Calibrator",
        options,
        Box::new(|_cc| Box::<MotocApp>::default()),
    )
}
