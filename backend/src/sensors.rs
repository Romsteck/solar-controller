use rppal::i2c::I2c;
use std::time::Duration;
use tokio::time;
use crate::state::{AppState, SensorReading};

const ADDR_A: u8 = 0x40;
const ADDR_B: u8 = 0x41;
const REG_SHUNT_VOLTAGE: u8 = 0x01;
const REG_BUS_VOLTAGE: u8 = 0x02;

fn read_reg(i2c: &mut I2c, reg: u8) -> anyhow::Result<i16> {
    let mut buf = [0u8; 2];
    i2c.write(&[reg])?;
    i2c.read(&mut buf)?;
    Ok(i16::from_be_bytes(buf))
}

fn read_sensor(i2c: &mut I2c, addr: u8) -> anyhow::Result<SensorReading> {
    i2c.set_slave_address(addr as u16)?;
    let bus_raw = read_reg(i2c, REG_BUS_VOLTAGE)?;
    let shunt_raw = read_reg(i2c, REG_SHUNT_VOLTAGE)?;
    // INA236 (joy-it SBC-DVA, Die ID 0xA080) : bus voltage = 16-bit unsigned, 1.6 mV/LSB.
    // Cast via u16 pour ne pas interpréter le MSB comme signe (range 0..81V).
    let bus_voltage_v = (bus_raw as u16 as f32) * 0.0016;
    // INA236 shunt voltage : signed 16-bit, 2.5 µV/LSB en ADCRANGE=0 (config 0x4127, default).
    // SBC-DVA utilise un shunt de 8 mΩ (cf. lib joy-it SBC_DVA_lib.py, SHUNT_CAL formula).
    // mA = (shunt_raw × 2.5 µV) / 0.008 Ω = shunt_raw × 312.5 µA = shunt_raw × 0.3125 mA
    let current_ma = (shunt_raw as f32) * 0.3125;
    Ok(SensorReading { address: addr, bus_voltage_v, current_ma })
}

pub async fn poll_loop(state: AppState) {
    let mut interval = time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        if let Ok(mut i2c) = I2c::new() {
            let readings: Vec<SensorReading> = [ADDR_A, ADDR_B]
                .iter()
                .filter_map(|&addr| read_sensor(&mut i2c, addr).ok())
                .collect();
            if !readings.is_empty() {
                // parking_lot::Mutex : `lock()` ne peut pas s'empoisonner et
                // ne retourne pas de Result, donc plus aucun `unwrap` ici.
                state.inner.lock().sensors = readings;
            }
        }
    }
}
