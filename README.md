# CubeLED Firmware Project

> This project is currently under development.

CubeLED is a firmware project for a LED cube made out of 6 PCB soldered in a phisical cube. This firmware is written in Rust. 
The PCB is based on an ESP32-C3-mini board, an ADXL362 accelerometer, a MAX17048 LiPo battery fuel gauge ICs and 25 addressable WS2812B 2020 size chip LEDS per side (150 LEDs in total).

The goal is to serve as a visual fidget toy, and a BLE device, which can notify the user or set a reminder.