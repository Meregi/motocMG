pub mod calibrator;
pub mod common;
pub mod helpers_xr;
pub mod mndx;
pub mod transformd;
pub mod cli;
pub mod gui;

use std::{
    collections::HashMap,
    env,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

use calibrator::{Calibrator, FloorMethod, Monitor, OffsetMethod, SampledMethod, StepResult};
use cli::{Args, Subcommands};
use common::{vec3, CalibratorData, Device, OffsetType, UNIT};
use indicatif::MultiProgress;
use libmonado as mnd;
use nalgebra::{Rotation3};
use openxr as xr;
use transformd::TransformD;

pub fn wait_monado() {
    let Ok(self_cmd) = env::current_exe() else {
        log::error!("Could not determine current exe.");
        return;
    };

    loop {
        match Command::new(&self_cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("check")
            .status()
        {
            Ok(e) if matches!(e.code(), Some(2)) => {}
            _ => {
                log::error!("Error while waiting for Monado!");
                break;
            }
        }

        thread::sleep(Duration::from_secs(5));
    }
}

pub fn handle_non_xr_subcommands(args: &Args, monado: &mnd::Monado) -> anyhow::Result<bool> {
    match &args.command {
        Subcommands::NumDevices => {
            let mut count = 0;
            for d in monado.devices()?.into_iter() {
                if !d.get_info_bool(mnd::MndProperty::PropertySupportsPositionBool)? {
                    continue;
                }
                count += 1;
            }
            println!("{}", count);
            Ok(true)
        }
        Subcommands::Show => {
            let device_list = get_device_list(monado)?;
            for (to_name, to_id, pose, devices) in device_list {
                println!("[{}] {}", to_id, to_name);
                let (roll, pitch, yaw) =
                    UnitQuaternion::from_rotation_matrix(&pose.basis)
                        .euler_angles();
                println!(
                    " │ POS: (X: {:.2}, Y: {:.2}, Z: {:.2})",
                    pose.origin.x, pose.origin.y, pose.origin.z
                );
                println!(" │ ROT: (Y: {:.2}, P: {:.2}, R: {:.2})", yaw, pitch, roll);

                let last = devices.len() - 1;
                for (i, (serial, name)) in devices.iter().enumerate() {
                    let branch = if last == i { '└' } else { '├' };
                    print!(" {}── \"{}\"", branch, serial);
                    if name != serial {
                        print!(" ({})", name);
                    }
                    println!();
                }
            }
            Ok(true)
        }
        Subcommands::Reset { ref id } => {
            match id.to_lowercase().as_str() {
                "stage" => {
                    monado.set_reference_space_offset(
                        mnd::ReferenceSpaceType::Stage,
                        TransformD::default().into(),
                    )?;
                    println!("STAGE has been reset!");
                }
                "local" => {
                    monado.set_reference_space_offset(
                        mnd::ReferenceSpaceType::Local,
                        TransformD::default().into(),
                    )?;
                    println!("LOCAL has been reset!");
                }
                a => {
                    let Ok(id) = a.parse::<u32>() else {
                        println!("ID must be a tracking origin ID or 'STAGE' or 'LOCAL'!");
                        return Ok(true);
                    };
                    for to in monado.tracking_origins()?.into_iter() {
                        if to.id != id {
                            continue;
                        }
                        match to.set_offset(TransformD::default().into()) {
                            Ok(_) => println!("{} has been reset.", to.name),
                            Err(e) => println!("Could not reset due to libmonado error: {:?}", e),
                        }
                        return Ok(true);
                    }
                    println!("No such tracking origin: {}", id);
                }
            }
            Ok(true)
        }
        Subcommands::Adjust {
            ref id,
            relative,
            yaw,
            x,
            y,
            z,
        } => {
            let id_lower = id.to_lowercase();

            let ref_space_type = match id_lower.as_str() {
                "stage" => Some(mnd::ReferenceSpaceType::Stage),
                "local" => Some(mnd::ReferenceSpaceType::Local),
                _ => None,
            };

            if let Some(ref_space_type) = ref_space_type {
                let mut offset = if *relative {
                    monado.get_reference_space_offset(ref_space_type)?.into()
                } else {
                    TransformD::default()
                };
                offset.origin += vec3(x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0));
                offset.basis =
                    Rotation3::from_axis_angle(&UNIT.YU, yaw.unwrap_or(0.0)) * offset.basis;

                match monado.set_reference_space_offset(ref_space_type, offset.into()) {
                    Ok(_) => println!("{:?} has been adjusted.", ref_space_type),
                    Err(e) => println!("Could not adjust due to libmonado error: {:?}", e),
                }
            } else {
                let maybe_id_num: Option<u32> = id.parse().ok();
                for to in monado.tracking_origins()?.into_iter() {
                    if maybe_id_num.is_none_or(|x| x != to.id) && id_lower != to.name.to_lowercase()
                    {
                        continue;
                    }

                    let mut offset = if *relative {
                        to.get_offset()?.into()
                    } else {
                        TransformD::default()
                    };

                    offset.origin += vec3(x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0));
                    offset.basis =
                        Rotation3::from_axis_angle(&UNIT.YU, yaw.unwrap_or(0.0)) * offset.basis;

                    match to.set_offset(offset.into()) {
                        Ok(_) => println!("{} has been adjusted.", to.name),
                        Err(e) => println!("Could not adjust due to libmonado error: {:?}", e),
                    }
                    break;
                }
            }

            Ok(true)
        }
        Subcommands::Check => Ok(true),
        _ => Ok(false),
    }
}

pub fn xr_loop(args: Args, monado: mnd::Monado, mut status: MultiProgress) -> anyhow::Result<()> {
    let (instance, system) = helpers_xr::xr_init()?;

    let actions = instance.create_action_set("motoc", "MoToC", 0)?;

    let (session, _, _) = unsafe {
        instance
            .create_session::<xr::headless::Headless>(system, &xr::headless::SessionCreateInfo {})?
    };

    session.attach_action_sets(&[&actions])?;

    let mndx = mndx::Mndx::new(&instance)?;

    log::info!("LibMonado API version {}", monado.get_api_version());

    let mut events = xr::EventDataBuffer::new();
    let mut session_running = false;

    let mut calibrator_data = None;
    let mut calibrator: Option<Box<dyn Calibrator>> = None;

    'main_loop: loop {
        'event_loop: while let Some(event) = instance.poll_event(&mut events)? {
            use xr::Event::*;
            match event {
                SessionStateChanged(e) => match e.state() {
                    xr::SessionState::READY => {
                        session.begin(xr::ViewConfigurationType::PRIMARY_STEREO)?;
                        session_running = true;
                        log::info!("XrSession started.");

                        if calibrator_data.is_some() {
                            continue 'event_loop;
                        }

                        let mut data = load_calibrator_data(&session, &mndx, &monado)?;

                        match &args.command {
                            Subcommands::Monitor => {
                                calibrator = Some(Box::new({
                                    let mut c = Monitor::new();
                                    c.init(&mut data, &mut status)?;
                                    c
                                }));
                            }
                            Subcommands::Offset {
                                ref src,
                                ref dst,
                                yaw,
                                pitch,
                                roll,
                                x,
                                y,
                                z,
                                lerp,
                            } => {
                                let Some(src_dev) = data.find_device(src) else {
                                    log::error!("src: no such device: {}", &src);
                                    break 'main_loop;
                                };
                                let Some(dst_dev) = data.find_device(dst) else {
                                    log::error!("dst: no such device: {}", &dst);
                                    break 'main_loop;
                                };

                                if data.devices[src_dev].tracking_origin
                                    == data.devices[dst_dev].tracking_origin
                                {
                                    log::error!("both devices are in the same tracking origin");
                                    break 'main_loop;
                                }

                                calibrator = Some(Box::new({
                                    let mut c = OffsetMethod::new(
                                        src_dev,
                                        dst_dev,
                                        vec3(
                                            pitch.unwrap_or(0.0),
                                            yaw.unwrap_or(0.0),
                                            roll.unwrap_or(0.0),
                                        ),
                                        vec3(x.unwrap_or(0.0), y.unwrap_or(0.0), z.unwrap_or(0.0)),
                                        *lerp,
                                    );
                                    c.init(&mut data, &mut status)?;
                                    c
                                }));
                            }
                            Subcommands::Calibrate {
                                ref src,
                                ref dst,
                                r#continue: maintain,
                                samples,
                                ref profile,
                            } => {
                                let Some(src_dev) = data.find_device(src) else {
                                    log::error!("src: no such device: {}", &src);
                                    break 'main_loop;
                                };
                                let Some(dst_dev) = data.find_device(dst) else {
                                    log::error!("dst: no such device: {}", &dst);
                                    break 'main_loop;
                                };

                                if data.devices[src_dev].tracking_origin
                                    == data.devices[dst_dev].tracking_origin
                                {
                                    log::error!("both devices are in the same tracking origin");
                                    break 'main_loop;
                                }

                                calibrator = Some(Box::new({
                                    let mut c = SampledMethod::new(
                                        src_dev,
                                        dst_dev,
                                        *maintain,
                                        samples.unwrap_or(500),
                                        profile.clone(),
                                    );
                                    c.init(&mut data, &mut status)?;
                                    c
                                }));
                            }
                            Subcommands::Continue { ref profile } => {
                                let Ok(last) = data.load_calibration(profile.as_str()) else {
                                    log::error!(
                                        "Could not load calibration for profile '{}'. Did you mean to calibrate first?",
                                        profile
                                    );
                                    break 'main_loop;
                                };

                                match last.offset_type {
                                    OffsetType::TrackingOrigin => {
                                        let src_transform = if let Some(src_origin) = data
                                            .tracking_origins
                                            .iter()
                                            .find(|x| x.name == last.src)
                                        {
                                            src_origin.get_offset()?.into()
                                        } else {
                                            log::warn!("Source origin \"{}\" not found, applying calibration with identity source", last.src);
                                            TransformD::default()
                                        };

                                        for o in data.tracking_origins.iter() {
                                            if o.name != last.dst {
                                                continue;
                                            }

                                            let offset = last.offset * src_transform;
                                            o.set_offset(offset.into())?;
                                            log::info!(
                                                "Offset successfully applied to: {}",
                                                last.dst
                                            );
                                            break 'main_loop;
                                        }
                                        log::error!("No such tracking origin: {}", last.dst);
                                    }
                                    OffsetType::Device => {
                                        let Some(src_idx) =
                                            data.devices.iter().position(|d| d.serial == last.src)
                                        else {
                                            log::error!("No such device: {}", last.src);
                                            break 'main_loop;
                                        };

                                        let Some(dst_idx) =
                                            data.devices.iter().position(|d| d.serial == last.dst)
                                        else {
                                            log::error!("No such device: {}", last.dst);
                                            break 'main_loop;
                                        };

                                        log::info!(
                                            "Starting continous mode from previous calibration."
                                        );

                                        calibrator = Some(Box::new({
                                            let mut c = OffsetMethod::new_internal(
                                                src_idx,
                                                dst_idx,
                                                last.offset,
                                                0.02,
                                            );
                                            c.init(&mut data, &mut status)?;
                                            c
                                        }));
                                    }
                                }
                            }
                            Subcommands::Floor => {
                                calibrator = Some(Box::new({
                                    let mut c = FloorMethod::new(&session)?;
                                    c.init(&mut data, &mut status)?;
                                    c
                                }));
                            }
                            _ => {}
                        }
                        calibrator_data = Some(data);
                    }
                    xr::SessionState::STOPPING => {
                        session.end()?;
                        session_running = false;
                        log::info!("XrSession stopped.")
                    }
                    xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                        anyhow::bail!("XR session exiting");
                    }
                    _ => {}
                },
                InstanceLossPending(_) => {
                    anyhow::bail!("XR instance loss pending");
                }
                EventsLost(e) => {
                    log::warn!("lost {} events", e.lost_event_count());
                }
                _ => {}
            }
        }

        if !session_running {
            thread::sleep(Duration::from_millis(40));
            continue 'main_loop;
        }

        session.sync_actions(&[(&actions).into()])?;

        if let (Some(data), Some(cal)) = (calibrator_data.as_mut(), calibrator.as_mut()) {
            data.now = instance.now()?;
            match cal.step(data)? {
                StepResult::End => {
                    status.clear()?;
                    log::info!("Our work here is done! ✅");
                    //TODO: have some interactibility
                    // calibrator = None;
                    break 'main_loop;
                }
                StepResult::Replace(mut new_calibrator) => {
                    status.clear()?;
                    new_calibrator.init(data, &mut status)?;
                    calibrator = Some(new_calibrator);
                }
                StepResult::Continue => {}
            }
        }

        thread::sleep(Duration::from_millis(40));
    }

    Ok(())
}

use nalgebra::{UnitQuaternion};

pub fn get_device_list(monado: &mnd::Monado) -> anyhow::Result<Vec<(String, u32, TransformD, Vec<(String, String)>)>> {
    let mut result = vec![];
    let mut devs = vec![];
    let mut dev_tos = vec![];

    for d in monado.devices()?.into_iter() {
        if !d.get_info_bool(mnd::MndProperty::PropertySupportsPositionBool)? {
            continue;
        }
        dev_tos.push(d.get_info_u32(mnd::MndProperty::PropertyTrackingOriginU32)?);
        devs.push(d);
    }

    for to in monado.tracking_origins()?.into_iter() {
        let mut to_devs_vec = vec![];
        let to_devs = devs
            .iter()
            .enumerate()
            .filter_map(|(i, d)| if dev_tos[i] == to.id { Some(d) } else { None })
            .collect::<Vec<_>>();

        for d in to_devs.iter() {
            let serial = d.serial()?;
            let name = if !d.name.is_empty() && d.name != serial {
                d.name.clone()
            } else {
                serial.clone()
            };
            to_devs_vec.push((serial, name));
        }
        let pose = to.get_offset()?.into();
        result.push((to.name.clone(), to.id, pose, to_devs_vec));
    }

    Ok(result)
}

fn load_calibrator_data<'a, G>(
    session: &xr::Session<G>,
    mndx: &mndx::Mndx,
    monado: &'a mnd::Monado,
) -> anyhow::Result<CalibratorData<'a>> {
    let mut devices = vec![];
    let mut tracking_origins = vec![];

    let mut serial_space = HashMap::new();

    let xdev_list = mndx.create_list(session)?;
    for xdev in xdev_list.enumerate_xdevs()?.into_iter() {
        if !xdev.can_create_space() {
            continue;
        }
        serial_space.insert(
            xdev.serial().to_string(),
            xdev.create_space(session.clone())?,
        );
    }

    for dev in monado.devices()?.into_iter() {
        let serial = dev.serial()?;
        let Some(space) = serial_space.remove(serial.as_str()) else {
            continue;
        };

        devices.push(Device {
            tracking_origin: dev.get_info_u32(mnd::MndProperty::PropertyTrackingOriginU32)?,
            serial,
            space,
            index: dev.index,
            inner: dev,
        });
    }

    for to in monado.tracking_origins()?.into_iter() {
        tracking_origins.push(to);
    }

    Ok(CalibratorData {
        monado,
        devices,
        tracking_origins,
        stage: session
            .create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)?,
        now: session.instance().now()?,
    })
}
