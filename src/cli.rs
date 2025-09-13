use std::{
    env,
    process::ExitCode,
};

use clap::Parser;
use env_logger::Env;
use indicatif::MultiProgress;
use indicatif_log_bridge::LogWrapper;
use libmonado as mnd;

use crate::{handle_non_xr_subcommands, wait_monado, xr_loop};

#[derive(clap::Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// The command to run
    #[command(subcommand)]
    pub command: Subcommands,

    /// Wait for Monado to become available (instead of exiting)
    #[arg(short, long)]
    pub wait: bool,
}

#[derive(clap::Parser, Debug)]
pub enum Subcommands {
    /// Show available tracking origings and their devices
    Show,
    /// Continuously monitor tracking origins and their devices
    Monitor,
    /// Maintain a static offset between two devices
    Offset {
        /// the source device (usu. HMD)
        #[arg(long, value_name = "SERIAL_NUMBER")]
        src: String,

        /// the destination device (usu. tracker)
        #[arg(long, value_name = "SERIAL_NUMBER")]
        dst: String,

        /// rotation offset from dst to src device in DEGREES
        #[arg(long)]
        yaw: Option<f64>,

        /// rotation offset from dst to src device in DEGREES
        #[arg(long)]
        pitch: Option<f64>,

        /// rotation offset from dst to src device in DEGREES
        #[arg(long)]
        roll: Option<f64>,

        /// position offset from dst to src device in METERS
        #[arg(long)]
        x: Option<f64>,

        /// position offset from dst to src device in METERS
        #[arg(long)]
        y: Option<f64>,

        /// position offset from dst to src device in METERS
        #[arg(long)]
        z: Option<f64>,

        /// interpolation factor, lower is smoother. range (0, 1]
        #[arg(long, value_name = "FACTOR", default_value = "0.05")]
        lerp: f64,
    },
    /// Calibrate by sampling two devices that move together over time
    Calibrate {
        /// the numeric id or serial number of the source device (usu. HMD)
        #[arg(long, value_name = "DEVICE")]
        src: String,

        /// the numeric id or serial number of the destination device (usu. tracker)
        #[arg(long, value_name = "DEVICE")]
        dst: String,

        /// continue maintaining offset after calibration. enable if the devices are firmly attached
        #[arg(long)]
        r#continue: bool,

        /// number of samples to use for initial calibration. default: 500
        #[arg(long)]
        samples: Option<u32>,

        /// save the calubration with this profile name
        #[arg(long, value_name = "NAME", default_value = "last")]
        profile: String,
    },
    /// Auto-adjust the floor level using hand tracking, by placing hands on floor
    Floor,
    /// Manually adjust the offset of the given tracking origin
    Adjust {
        /// tracking origin ID from `motoc show`
        #[arg(value_name = "ORIGIN")]
        id: String,

        /// apply a relative offset instead of overriding the existing one
        #[arg(short, long)]
        relative: bool,

        /// rotation offset, positive is clockwise
        #[arg(long, value_name = "DEGREES")]
        yaw: Option<f64>,

        /// position offset
        #[arg(long, value_name = "METERS")]
        x: Option<f64>,

        /// position offset
        #[arg(long, value_name = "METERS")]
        y: Option<f64>,

        /// position offset
        #[arg(long, value_name = "METERS")]
        z: Option<f64>,
    },
    /// Reset the offset for the given tracking origin.
    Reset {
        /// tracking origin ID from `motoc show` or 'STAGE' or 'LOCAL'
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Load a previous calibration. If last calibration was not continous; apply once and exit.
    Continue {
        /// load the calubration from this profile
        #[arg(long, value_name = "NAME", default_value = "last")]
        profile: String,
    },
    /// Check if Monado is reachable, then exit.
    Check,
    /// Return the number of discovered devices
    NumDevices,
}

pub fn run() -> ExitCode {
    let log = env_logger::Builder::from_env(Env::default().default_filter_or("info")).build();
    let status = MultiProgress::new();
    LogWrapper::new(status.clone(), log).try_init().unwrap();

    let args = Args::parse();

    if args.wait {
        log::info!("Waiting for Monado to become reachable...");
        wait_monado();
    }

    let Ok(monado) = mnd::Monado::auto_connect() else {
        if !args.wait {
            log::error!("Monado is not reachable.");

            let cmd = env::args().skip(1).fold(String::new(), |a, b| a + " " + &b);
            log::error!("Want to wait until Monado is available?");
            log::error!("Try: motoc --wait{}", cmd);
        }
        return ExitCode::from(2);
    };

    let required_libmonado_version = mnd::Version::new(1, 4, 0);
    let libmonado_version = monado.get_api_version();
    if libmonado_version < required_libmonado_version {
        log::error!("Your libmonado API version is not supported.");
        log::error!("Please update your Monado/WiVRn installation.");
        log::error!("Required: API {} or later", required_libmonado_version);
        log::error!("Current: API {}", libmonado_version);
        return ExitCode::FAILURE;
    }

    match handle_non_xr_subcommands(&args, &monado) {
        Ok(true) => return ExitCode::SUCCESS,
        Ok(false) => {}
        Err(e) => {
            log::error!("{:?}", e);
            return ExitCode::FAILURE;
        }
    }

    if let Err(e) = xr_loop(args, monado, status) {
        log::error!("{:?}", e);
        // return;
    }

    ExitCode::SUCCESS
}
