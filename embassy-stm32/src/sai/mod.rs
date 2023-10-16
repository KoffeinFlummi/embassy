#![macro_use]

use embassy_embedded_hal::SetConfig;
use embassy_hal_internal::{into_ref, PeripheralRef};

pub use crate::dma::word;
use crate::dma::{ringbuffer, Channel, ReadableRingBuffer, Request, TransferOptions, WritableRingBuffer};
use crate::gpio::sealed::{AFType, Pin as _};
use crate::gpio::AnyPin;
use crate::pac::sai::{vals, Sai as Regs};
use crate::rcc::RccPeripheral;
use crate::{peripherals, Peripheral};

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum Error {
    NotATransmitter,
    NotAReceiver,
    OverrunError,
}

impl From<ringbuffer::OverrunError> for Error {
    fn from(_: ringbuffer::OverrunError) -> Self {
        Self::OverrunError
    }
}

#[derive(Copy, Clone)]
pub enum SyncBlock {
    None,
    Sai1BlockA,
    Sai1BlockB,
    Sai2BlockA,
    Sai2BlockB,
}

#[derive(Copy, Clone)]
pub enum SyncIn {
    None,
    ChannelZero,
    ChannelOne,
}

#[derive(Copy, Clone)]
pub enum Mode {
    Master,
    Slave,
}

#[derive(Copy, Clone)]
pub enum TxRx {
    Transmitter,
    Receiver,
}

impl Mode {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    const fn mode(&self, tx_rx: TxRx) -> vals::Mode {
        match tx_rx {
            TxRx::Transmitter => match self {
                Mode::Master => vals::Mode::MASTERTX,
                Mode::Slave => vals::Mode::SLAVETX,
            },
            TxRx::Receiver => match self {
                Mode::Master => vals::Mode::MASTERRX,
                Mode::Slave => vals::Mode::SLAVERX,
            },
        }
    }
}

#[derive(Copy, Clone)]
pub enum SlotSize {
    DataSize,
    /// 16 bit data length on 16 bit wide channel
    Channel16,
    /// 16 bit data length on 32 bit wide channel
    Channel32,
}

impl SlotSize {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn slotsz(&self) -> vals::Slotsz {
        match self {
            SlotSize::DataSize => vals::Slotsz::DATASIZE,
            SlotSize::Channel16 => vals::Slotsz::BIT16,
            SlotSize::Channel32 => vals::Slotsz::BIT32,
        }
    }
}

#[derive(Copy, Clone)]
pub enum DataSize {
    Data8,
    Data10,
    Data16,
    Data20,
    Data24,
    Data32,
}

impl DataSize {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn ds(&self) -> vals::Ds {
        match self {
            DataSize::Data8 => vals::Ds::BIT8,
            DataSize::Data10 => vals::Ds::BIT10,
            DataSize::Data16 => vals::Ds::BIT16,
            DataSize::Data20 => vals::Ds::BIT20,
            DataSize::Data24 => vals::Ds::BIT24,
            DataSize::Data32 => vals::Ds::BIT32,
        }
    }
}

#[derive(Copy, Clone)]
pub enum FifoThreshold {
    Empty,
    Quarter,
    Half,
    ThreeQuarters,
    Full,
}

impl FifoThreshold {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn fth(&self) -> vals::Fth {
        match self {
            FifoThreshold::Empty => vals::Fth::EMPTY,
            FifoThreshold::Quarter => vals::Fth::QUARTER1,
            FifoThreshold::Half => vals::Fth::QUARTER2,
            FifoThreshold::ThreeQuarters => vals::Fth::QUARTER3,
            FifoThreshold::Full => vals::Fth::FULL,
        }
    }
}

#[derive(Copy, Clone)]
pub enum FifoLevel {
    Empty,
    FirstQuarter,
    SecondQuarter,
    ThirdQuarter,
    FourthQuarter,
    Full,
}

#[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
impl From<vals::Flvl> for FifoLevel {
    fn from(flvl: vals::Flvl) -> Self {
        match flvl {
            vals::Flvl::EMPTY => FifoLevel::Empty,
            vals::Flvl::QUARTER1 => FifoLevel::FirstQuarter,
            vals::Flvl::QUARTER2 => FifoLevel::SecondQuarter,
            vals::Flvl::QUARTER3 => FifoLevel::ThirdQuarter,
            vals::Flvl::QUARTER4 => FifoLevel::FourthQuarter,
            vals::Flvl::FULL => FifoLevel::Full,
            _ => FifoLevel::Empty,
        }
    }
}

#[derive(Copy, Clone)]
pub enum MuteDetection {
    NoMute,
    Mute,
}

#[derive(Copy, Clone)]
pub enum MuteValue {
    Zero,
    LastValue,
}

impl MuteValue {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn muteval(&self) -> vals::Muteval {
        match self {
            MuteValue::Zero => vals::Muteval::SENDZERO,
            MuteValue::LastValue => vals::Muteval::SENDLAST,
        }
    }
}

#[derive(Copy, Clone)]
pub enum OverUnderStatus {
    NoError,
    OverUnderRunDetected,
}

#[derive(Copy, Clone)]
pub enum Protocol {
    Free,
    Spdif,
    Ac97,
}

impl Protocol {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn prtcfg(&self) -> vals::Prtcfg {
        match self {
            Protocol::Free => vals::Prtcfg::FREE,
            Protocol::Spdif => vals::Prtcfg::SPDIF,
            Protocol::Ac97 => vals::Prtcfg::AC97,
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum SyncEnable {
    Asynchronous,
    /// Syncs with the other A/B sub-block within the SAI unit
    Internal,
    /// Syncs with a sub-block in the other SAI unit - use set_sync_output() and set_sync_input()
    #[cfg(any(sai_v4))]
    External,
}

impl SyncEnable {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn syncen(&self) -> vals::Syncen {
        match self {
            SyncEnable::Asynchronous => vals::Syncen::ASYNCHRONOUS,
            SyncEnable::Internal => vals::Syncen::INTERNAL,
            #[cfg(any(sai_v4))]
            SyncEnable::External => vals::Syncen::EXTERNAL,
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum StereoMono {
    Stereo,
    Mono,
}

impl StereoMono {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn mono(&self) -> vals::Mono {
        match self {
            StereoMono::Stereo => vals::Mono::STEREO,
            StereoMono::Mono => vals::Mono::MONO,
        }
    }
}

#[derive(Copy, Clone)]
pub enum BitOrder {
    LsbFirst,
    MsbFirst,
}

impl BitOrder {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn lsbfirst(&self) -> vals::Lsbfirst {
        match self {
            BitOrder::LsbFirst => vals::Lsbfirst::LSBFIRST,
            BitOrder::MsbFirst => vals::Lsbfirst::MSBFIRST,
        }
    }
}

#[derive(Copy, Clone)]
pub enum FrameSyncOffset {
    /// This is used in modes other than standard I2S phillips mode
    OnFirstBit,
    /// This is used in standard I2S phillips mode
    BeforeFirstBit,
}

impl FrameSyncOffset {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn fsoff(&self) -> vals::Fsoff {
        match self {
            FrameSyncOffset::OnFirstBit => vals::Fsoff::ONFIRST,
            FrameSyncOffset::BeforeFirstBit => vals::Fsoff::BEFOREFIRST,
        }
    }
}

#[derive(Copy, Clone)]
pub enum FrameSyncPolarity {
    ActiveLow,
    ActiveHigh,
}

impl FrameSyncPolarity {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn fspol(&self) -> vals::Fspol {
        match self {
            FrameSyncPolarity::ActiveLow => vals::Fspol::FALLINGEDGE,
            FrameSyncPolarity::ActiveHigh => vals::Fspol::RISINGEDGE,
        }
    }
}

#[derive(Copy, Clone)]
pub enum FrameSyncDefinition {
    StartOfFrame,
    ChannelIdentification,
}

impl FrameSyncDefinition {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn fsdef(&self) -> bool {
        match self {
            FrameSyncDefinition::StartOfFrame => false,
            FrameSyncDefinition::ChannelIdentification => true,
        }
    }
}

#[derive(Copy, Clone)]
pub enum ClockStrobe {
    Falling,
    Rising,
}

impl ClockStrobe {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn ckstr(&self) -> vals::Ckstr {
        match self {
            ClockStrobe::Falling => vals::Ckstr::FALLINGEDGE,
            ClockStrobe::Rising => vals::Ckstr::RISINGEDGE,
        }
    }
}

#[derive(Copy, Clone)]
pub enum ComplementFormat {
    OnesComplement,
    TwosComplement,
}

impl ComplementFormat {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn cpl(&self) -> vals::Cpl {
        match self {
            ComplementFormat::OnesComplement => vals::Cpl::ONESCOMPLEMENT,
            ComplementFormat::TwosComplement => vals::Cpl::TWOSCOMPLEMENT,
        }
    }
}

#[derive(Copy, Clone)]
pub enum Companding {
    None,
    MuLaw,
    ALaw,
}

impl Companding {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn comp(&self) -> vals::Comp {
        match self {
            Companding::None => vals::Comp::NOCOMPANDING,
            Companding::MuLaw => vals::Comp::MULAW,
            Companding::ALaw => vals::Comp::ALAW,
        }
    }
}

#[derive(Copy, Clone)]
pub enum OutputDrive {
    OnStart,
    Immediately,
}

impl OutputDrive {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn outdriv(&self) -> vals::Outdriv {
        match self {
            OutputDrive::OnStart => vals::Outdriv::ONSTART,
            OutputDrive::Immediately => vals::Outdriv::IMMEDIATELY,
        }
    }
}

#[derive(Copy, Clone, PartialEq)]
pub enum MasterClockDivider {
    MasterClockDisabled,
    Div1,
    Div2,
    Div4,
    Div6,
    Div8,
    Div10,
    Div12,
    Div14,
    Div16,
    Div18,
    Div20,
    Div22,
    Div24,
    Div26,
    Div28,
    Div30,
}

impl MasterClockDivider {
    #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
    pub const fn mckdiv(&self) -> u8 {
        match self {
            MasterClockDivider::MasterClockDisabled => 0,
            MasterClockDivider::Div1 => 0,
            MasterClockDivider::Div2 => 1,
            MasterClockDivider::Div4 => 2,
            MasterClockDivider::Div6 => 3,
            MasterClockDivider::Div8 => 4,
            MasterClockDivider::Div10 => 5,
            MasterClockDivider::Div12 => 6,
            MasterClockDivider::Div14 => 7,
            MasterClockDivider::Div16 => 8,
            MasterClockDivider::Div18 => 9,
            MasterClockDivider::Div20 => 10,
            MasterClockDivider::Div22 => 11,
            MasterClockDivider::Div24 => 12,
            MasterClockDivider::Div26 => 13,
            MasterClockDivider::Div28 => 14,
            MasterClockDivider::Div30 => 15,
        }
    }
}

/// [`SAI`] configuration.
#[non_exhaustive]
#[derive(Copy, Clone)]
pub struct Config {
    pub mode: Mode,
    pub tx_rx: TxRx,
    pub sync_enable: SyncEnable,
    pub is_sync_output: bool,
    pub protocol: Protocol,
    pub slot_size: SlotSize,
    pub slot_count: word::U4,
    pub slot_enable: u16,
    pub first_bit_offset: word::U5,
    pub data_size: DataSize,
    pub stereo_mono: StereoMono,
    pub bit_order: BitOrder,
    pub frame_sync_offset: FrameSyncOffset,
    pub frame_sync_polarity: FrameSyncPolarity,
    pub frame_sync_active_level_length: word::U7,
    pub frame_sync_definition: FrameSyncDefinition,
    pub frame_length: u8,
    pub clock_strobe: ClockStrobe,
    pub output_drive: OutputDrive,
    pub master_clock_divider: MasterClockDivider,
    pub is_high_impedenane_on_inactive_slot: bool,
    pub fifo_threshold: FifoThreshold,
    pub companding: Companding,
    pub complement_format: ComplementFormat,
    pub mute_value: MuteValue,
    pub mute_detection_counter: word::U5,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::Master,
            tx_rx: TxRx::Transmitter,
            is_sync_output: false,
            sync_enable: SyncEnable::Asynchronous,
            protocol: Protocol::Free,
            slot_size: SlotSize::DataSize,
            slot_count: word::U4(2),
            first_bit_offset: word::U5(0),
            slot_enable: 0b11,
            data_size: DataSize::Data16,
            stereo_mono: StereoMono::Stereo,
            bit_order: BitOrder::LsbFirst,
            frame_sync_offset: FrameSyncOffset::BeforeFirstBit,
            frame_sync_polarity: FrameSyncPolarity::ActiveLow,
            frame_sync_active_level_length: word::U7(16),
            frame_sync_definition: FrameSyncDefinition::ChannelIdentification,
            frame_length: 32,
            master_clock_divider: MasterClockDivider::MasterClockDisabled,
            clock_strobe: ClockStrobe::Rising,
            output_drive: OutputDrive::Immediately,
            is_high_impedenane_on_inactive_slot: false,
            fifo_threshold: FifoThreshold::ThreeQuarters,
            companding: Companding::None,
            complement_format: ComplementFormat::TwosComplement,
            mute_value: MuteValue::Zero,
            mute_detection_counter: word::U5(4),
        }
    }
}

impl Config {
    pub fn new_i2s() -> Self {
        return Default::default();
    }

    pub fn new_msb_first() -> Self {
        Self {
            bit_order: BitOrder::MsbFirst,
            frame_sync_offset: FrameSyncOffset::OnFirstBit,
            ..Default::default()
        }
    }
}

#[derive(Copy, Clone)]
enum WhichSubBlock {
    A = 0,
    B = 1,
}

enum RingBuffer<'d, C: Channel, W: word::Word> {
    Writable(WritableRingBuffer<'d, C, W>),
    Readable(ReadableRingBuffer<'d, C, W>),
}

#[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
fn dr<W: word::Word>(w: crate::pac::sai::Sai, sub_block: WhichSubBlock) -> *mut W {
    let ch = w.ch(sub_block as usize);
    ch.dr().as_ptr() as _
}

pub struct SubBlock<'d, T: Instance, C: Channel, W: word::Word> {
    _peri: PeripheralRef<'d, T>,
    sd: Option<PeripheralRef<'d, AnyPin>>,
    fs: Option<PeripheralRef<'d, AnyPin>>,
    sck: Option<PeripheralRef<'d, AnyPin>>,
    mclk: Option<PeripheralRef<'d, AnyPin>>,
    ring_buffer: RingBuffer<'d, C, W>,
    sub_block: WhichSubBlock,
}

pub struct SubBlockA {}
pub struct SubBlockB {}

pub struct SubBlockAPeripheral<'d, T>(PeripheralRef<'d, T>);
pub struct SubBlockBPeripheral<'d, T>(PeripheralRef<'d, T>);

pub struct Sai<'d, T: Instance> {
    _peri: PeripheralRef<'d, T>,
    sub_block_a_peri: Option<SubBlockAPeripheral<'d, T>>,
    sub_block_b_peri: Option<SubBlockBPeripheral<'d, T>>,
}

// return the type for (sd, sck)
fn get_af_types(mode: Mode, tx_rx: TxRx) -> (AFType, AFType) {
    (
        //sd is defined by tx/rx mode
        match tx_rx {
            TxRx::Transmitter => AFType::OutputPushPull,
            TxRx::Receiver => AFType::Input,
        },
        //clocks (mclk, sck and fs) are defined by master/slave
        match mode {
            Mode::Master => AFType::OutputPushPull,
            Mode::Slave => AFType::Input,
        },
    )
}

fn get_ring_buffer<'d, T: Instance, C: Channel, W: word::Word>(
    dma: impl Peripheral<P = C> + 'd,
    dma_buf: &'d mut [W],
    request: Request,
    sub_block: WhichSubBlock,
    tx_rx: TxRx,
) -> RingBuffer<'d, C, W> {
    let opts = TransferOptions {
        half_transfer_ir: true,
        //the new_write() and new_read() always use circular mode
        ..Default::default()
    };
    match tx_rx {
        TxRx::Transmitter => RingBuffer::Writable(unsafe {
            WritableRingBuffer::new_write(dma, request, dr(T::REGS, sub_block), dma_buf, opts)
        }),
        TxRx::Receiver => RingBuffer::Readable(unsafe {
            ReadableRingBuffer::new_read(dma, request, dr(T::REGS, sub_block), dma_buf, opts)
        }),
    }
}

impl<'d, T: Instance> Sai<'d, T> {
    pub fn new(peri: impl Peripheral<P = T> + 'd) -> Self {
        T::enable_and_reset();

        Self {
            _peri: unsafe { peri.clone_unchecked().into_ref() },
            sub_block_a_peri: Some(SubBlockAPeripheral(unsafe { peri.clone_unchecked().into_ref() })),
            sub_block_b_peri: Some(SubBlockBPeripheral(peri.into_ref())),
        }
    }

    pub fn take_sub_block_a(self: &mut Self) -> Option<SubBlockAPeripheral<'d, T>> {
        if self.sub_block_a_peri.is_some() {
            self.sub_block_a_peri.take()
        } else {
            None
        }
    }

    pub fn take_sub_block_b(self: &mut Self) -> Option<SubBlockBPeripheral<'d, T>> {
        if self.sub_block_b_peri.is_some() {
            self.sub_block_b_peri.take()
        } else {
            None
        }
    }
}

fn update_synchronous_config(config: &mut Config) {
    config.mode = Mode::Slave;
    config.is_sync_output = false;

    #[cfg(any(sai_v1, sai_v2, sai_v3))]
    {
        config.sync_enable = SyncEnable::Internal;
    }

    #[cfg(any(sai_v4))]
    {
        //this must either be Internal or External
        //The asynchronous sub-block on the same SAI needs to enable is_sync_output
        assert!(config.sync_enable != SyncEnable::Asynchronous);
    }
}

impl SubBlockA {
    pub fn new_asynchronous_with_mclk<'d, T: Instance, C: Channel, W: word::Word>(
        peri: SubBlockAPeripheral<'d, T>,
        sck: impl Peripheral<P = impl SckAPin<T>> + 'd,
        sd: impl Peripheral<P = impl SdAPin<T>> + 'd,
        fs: impl Peripheral<P = impl FsAPin<T>> + 'd,
        mclk: impl Peripheral<P = impl MclkAPin<T>> + 'd,
        dma: impl Peripheral<P = C> + 'd,
        dma_buf: &'d mut [W],
        mut config: Config,
    ) -> SubBlock<'d, T, C, W>
    where
        C: Channel + DmaA<T>,
    {
        into_ref!(mclk);

        let (_sd_af_type, ck_af_type) = get_af_types(config.mode, config.tx_rx);

        mclk.set_as_af(mclk.af_num(), ck_af_type);
        mclk.set_speed(crate::gpio::Speed::VeryHigh);

        if config.master_clock_divider == MasterClockDivider::MasterClockDisabled {
            config.master_clock_divider = MasterClockDivider::Div1;
        }

        Self::new_asynchronous(peri, sck, sd, fs, dma, dma_buf, config)
    }

    pub fn new_asynchronous<'d, T: Instance, C: Channel, W: word::Word>(
        peri: SubBlockAPeripheral<'d, T>,
        sck: impl Peripheral<P = impl SckAPin<T>> + 'd,
        sd: impl Peripheral<P = impl SdAPin<T>> + 'd,
        fs: impl Peripheral<P = impl FsAPin<T>> + 'd,
        dma: impl Peripheral<P = C> + 'd,
        dma_buf: &'d mut [W],
        config: Config,
    ) -> SubBlock<'d, T, C, W>
    where
        C: Channel + DmaA<T>,
    {
        let peri = peri.0;
        into_ref!(peri, dma, sck, sd, fs);

        let (sd_af_type, ck_af_type) = get_af_types(config.mode, config.tx_rx);
        sd.set_as_af(sd.af_num(), sd_af_type);
        sd.set_speed(crate::gpio::Speed::VeryHigh);

        sck.set_as_af(sck.af_num(), ck_af_type);
        sck.set_speed(crate::gpio::Speed::VeryHigh);
        fs.set_as_af(fs.af_num(), ck_af_type);
        fs.set_speed(crate::gpio::Speed::VeryHigh);

        let sub_block = WhichSubBlock::A;
        let request = dma.request();

        SubBlock::new_inner(
            peri,
            sub_block,
            Some(sck.map_into()),
            None,
            Some(sd.map_into()),
            Some(fs.map_into()),
            get_ring_buffer::<T, C, W>(dma, dma_buf, request, sub_block, config.tx_rx),
            config,
        )
    }

    pub fn new_synchronous<'d, T: Instance, C: Channel, W: word::Word>(
        peri: SubBlockAPeripheral<'d, T>,
        sd: impl Peripheral<P = impl SdAPin<T>> + 'd,
        dma: impl Peripheral<P = C> + 'd,
        dma_buf: &'d mut [W],
        mut config: Config,
    ) -> SubBlock<'d, T, C, W>
    where
        C: Channel + DmaA<T>,
    {
        update_synchronous_config(&mut config);

        let peri = peri.0;
        into_ref!(dma, peri, sd);

        let (sd_af_type, _ck_af_type) = get_af_types(config.mode, config.tx_rx);

        sd.set_as_af(sd.af_num(), sd_af_type);
        sd.set_speed(crate::gpio::Speed::VeryHigh);

        let sub_block = WhichSubBlock::A;
        let request = dma.request();

        SubBlock::new_inner(
            peri,
            sub_block,
            None,
            None,
            Some(sd.map_into()),
            None,
            get_ring_buffer::<T, C, W>(dma, dma_buf, request, sub_block, config.tx_rx),
            config,
        )
    }
}

impl SubBlockB {
    pub fn new_asynchronous_with_mclk<'d, T: Instance, C: Channel, W: word::Word>(
        peri: SubBlockBPeripheral<'d, T>,
        sck: impl Peripheral<P = impl SckBPin<T>> + 'd,
        sd: impl Peripheral<P = impl SdBPin<T>> + 'd,
        fs: impl Peripheral<P = impl FsBPin<T>> + 'd,
        mclk: impl Peripheral<P = impl MclkBPin<T>> + 'd,
        dma: impl Peripheral<P = C> + 'd,
        dma_buf: &'d mut [W],
        mut config: Config,
    ) -> SubBlock<'d, T, C, W>
    where
        C: Channel + DmaB<T>,
    {
        into_ref!(mclk);

        let (_sd_af_type, ck_af_type) = get_af_types(config.mode, config.tx_rx);

        mclk.set_as_af(mclk.af_num(), ck_af_type);
        mclk.set_speed(crate::gpio::Speed::VeryHigh);

        if config.master_clock_divider == MasterClockDivider::MasterClockDisabled {
            config.master_clock_divider = MasterClockDivider::Div1;
        }

        Self::new_asynchronous(peri, sck, sd, fs, dma, dma_buf, config)
    }

    pub fn new_asynchronous<'d, T: Instance, C: Channel, W: word::Word>(
        peri: SubBlockBPeripheral<'d, T>,
        sck: impl Peripheral<P = impl SckBPin<T>> + 'd,
        sd: impl Peripheral<P = impl SdBPin<T>> + 'd,
        fs: impl Peripheral<P = impl FsBPin<T>> + 'd,
        dma: impl Peripheral<P = C> + 'd,
        dma_buf: &'d mut [W],
        config: Config,
    ) -> SubBlock<'d, T, C, W>
    where
        C: Channel + DmaB<T>,
    {
        let peri = peri.0;
        into_ref!(dma, peri, sck, sd, fs);

        let (sd_af_type, ck_af_type) = get_af_types(config.mode, config.tx_rx);

        sd.set_as_af(sd.af_num(), sd_af_type);
        sd.set_speed(crate::gpio::Speed::VeryHigh);

        sck.set_as_af(sck.af_num(), ck_af_type);
        sck.set_speed(crate::gpio::Speed::VeryHigh);
        fs.set_as_af(fs.af_num(), ck_af_type);
        fs.set_speed(crate::gpio::Speed::VeryHigh);

        let sub_block = WhichSubBlock::B;
        let request = dma.request();

        SubBlock::new_inner(
            peri,
            sub_block,
            Some(sck.map_into()),
            None,
            Some(sd.map_into()),
            Some(fs.map_into()),
            get_ring_buffer::<T, C, W>(dma, dma_buf, request, sub_block, config.tx_rx),
            config,
        )
    }

    pub fn new_synchronous<'d, T: Instance, C: Channel, W: word::Word>(
        peri: SubBlockBPeripheral<'d, T>,
        sd: impl Peripheral<P = impl SdBPin<T>> + 'd,
        dma: impl Peripheral<P = C> + 'd,
        dma_buf: &'d mut [W],
        mut config: Config,
    ) -> SubBlock<'d, T, C, W>
    where
        C: Channel + DmaB<T>,
    {
        update_synchronous_config(&mut config);
        let peri = peri.0;
        into_ref!(dma, peri, sd);

        let (sd_af_type, _ck_af_type) = get_af_types(config.mode, config.tx_rx);

        sd.set_as_af(sd.af_num(), sd_af_type);
        sd.set_speed(crate::gpio::Speed::VeryHigh);

        let sub_block = WhichSubBlock::B;
        let request = dma.request();

        SubBlock::new_inner(
            peri,
            sub_block,
            None,
            None,
            Some(sd.map_into()),
            None,
            get_ring_buffer::<T, C, W>(dma, dma_buf, request, sub_block, config.tx_rx),
            config,
        )
    }
}

impl<'d, T: Instance, C: Channel, W: word::Word> SubBlock<'d, T, C, W> {
    pub fn start(self: &mut Self) {
        match self.ring_buffer {
            RingBuffer::Writable(ref mut rb) => {
                rb.start();
            }
            RingBuffer::Readable(ref mut rb) => {
                rb.start();
            }
        }
    }

    fn is_transmitter(ring_buffer: &RingBuffer<C, W>) -> bool {
        match ring_buffer {
            RingBuffer::Writable(_) => true,
            _ => false,
        }
    }

    fn new_inner(
        peri: impl Peripheral<P = T> + 'd,
        sub_block: WhichSubBlock,
        sck: Option<PeripheralRef<'d, AnyPin>>,
        mclk: Option<PeripheralRef<'d, AnyPin>>,
        sd: Option<PeripheralRef<'d, AnyPin>>,
        fs: Option<PeripheralRef<'d, AnyPin>>,
        ring_buffer: RingBuffer<'d, C, W>,
        config: Config,
    ) -> Self {
        #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
        {
            let ch = T::REGS.ch(sub_block as usize);
            ch.cr1().modify(|w| w.set_saien(false));
        }

        #[cfg(any(sai_v4))]
        {
            // Not totally clear from the datasheet if this is right
            // This is only used if using SyncEnable::External on the other SAI unit
            // Syncing from SAIX subblock A to subblock B does not require this
            // Only syncing from SAI1 subblock A/B to SAI2 subblock A/B
            let value: u8 = if T::REGS.as_ptr() == stm32_metapac::SAI1.as_ptr() {
                1 //this is SAI1, so sync with SAI2
            } else {
                0 //this is SAI2, so sync with SAI1
            };
            T::REGS.gcr().modify(|w| {
                w.set_syncin(value);
            });

            if config.is_sync_output {
                let syncout: u8 = match sub_block {
                    WhichSubBlock::A => 0b01,
                    WhichSubBlock::B => 0b10,
                };
                T::REGS.gcr().modify(|w| {
                    w.set_syncout(syncout);
                });
            }
        }

        #[cfg(any(sai_v1, sai_v2, sai_v3, sai_v4))]
        {
            let ch = T::REGS.ch(sub_block as usize);
            ch.cr1().modify(|w| {
                w.set_mode(config.mode.mode(if Self::is_transmitter(&ring_buffer) {
                    TxRx::Transmitter
                } else {
                    TxRx::Receiver
                }));
                w.set_prtcfg(config.protocol.prtcfg());
                w.set_ds(config.data_size.ds());
                w.set_lsbfirst(config.bit_order.lsbfirst());
                w.set_ckstr(config.clock_strobe.ckstr());
                w.set_syncen(config.sync_enable.syncen());
                w.set_mono(config.stereo_mono.mono());
                w.set_outdriv(config.output_drive.outdriv());
                w.set_mckdiv(config.master_clock_divider.mckdiv());
                w.set_nodiv(
                    if config.master_clock_divider == MasterClockDivider::MasterClockDisabled {
                        vals::Nodiv::NODIV
                    } else {
                        vals::Nodiv::MASTERCLOCK
                    },
                );
                w.set_dmaen(true);
            });

            ch.cr2().modify(|w| {
                w.set_fth(config.fifo_threshold.fth());
                w.set_comp(config.companding.comp());
                w.set_cpl(config.complement_format.cpl());
                w.set_muteval(config.mute_value.muteval());
                w.set_mutecnt(config.mute_detection_counter.0 as u8);
                w.set_tris(config.is_high_impedenane_on_inactive_slot);
            });

            ch.frcr().modify(|w| {
                w.set_fsoff(config.frame_sync_offset.fsoff());
                w.set_fspol(config.frame_sync_polarity.fspol());
                w.set_fsdef(config.frame_sync_definition.fsdef());
                w.set_fsall(config.frame_sync_active_level_length.0 as u8 - 1);
                w.set_frl(config.frame_length - 1);
            });

            ch.slotr().modify(|w| {
                w.set_nbslot(config.slot_count.0 as u8 - 1);
                w.set_slotsz(config.slot_size.slotsz());
                w.set_fboff(config.first_bit_offset.0 as u8);
                w.set_sloten(vals::Sloten(config.slot_enable as u16));
            });

            ch.cr1().modify(|w| w.set_saien(true));

            if ch.cr1().read().saien() == false {
                panic!("SAI failed to enable. Check that config is valid (frame length, slot count, etc)");
            }
        }

        Self {
            _peri: peri.into_ref(),
            sub_block,
            sck,
            mclk,
            sd,
            fs,
            ring_buffer,
        }
    }

    pub fn reset() {
        T::enable_and_reset();
    }

    pub fn flush(&mut self) {
        let ch = T::REGS.ch(self.sub_block as usize);
        ch.cr1().modify(|w| w.set_saien(false));
        #[cfg(any(sai_v1, sai_v2))]
        {
            ch.cr2().modify(|w| w.set_fflush(vals::Fflush::FLUSH));
        }
        #[cfg(any(sai_v3, sai_v4))]
        {
            ch.cr2().modify(|w| w.set_fflush(true));
        }
        ch.cr1().modify(|w| w.set_saien(true));
    }

    pub fn set_mute(&mut self, value: bool) {
        let ch = T::REGS.ch(self.sub_block as usize);
        ch.cr2().modify(|w| w.set_mute(value));
    }

    #[allow(dead_code)]
    /// Reconfigures it with the supplied config.
    fn reconfigure(&mut self, _config: Config) {}

    pub fn get_current_config(&self) -> Config {
        Config::default()
    }

    pub async fn write(&mut self, data: &[W]) -> Result<(), Error> {
        match &mut self.ring_buffer {
            RingBuffer::Writable(buffer) => {
                buffer.write_exact(data).await?;
                Ok(())
            }
            _ => return Err(Error::NotATransmitter),
        }
    }

    pub async fn read(&mut self, data: &mut [W]) -> Result<(), Error> {
        match &mut self.ring_buffer {
            RingBuffer::Readable(buffer) => {
                buffer.read_exact(data).await?;
                Ok(())
            }
            _ => Err(Error::NotAReceiver),
        }
    }
}

impl<'d, T: Instance, C: Channel, W: word::Word> Drop for SubBlock<'d, T, C, W> {
    fn drop(&mut self) {
        let ch = T::REGS.ch(self.sub_block as usize);
        ch.cr1().modify(|w| w.set_saien(false));
        self.fs.as_ref().map(|x| x.set_as_disconnected());
        self.sd.as_ref().map(|x| x.set_as_disconnected());
        self.sck.as_ref().map(|x| x.set_as_disconnected());
        self.mclk.as_ref().map(|x| x.set_as_disconnected());
    }
}

pub(crate) mod sealed {
    use super::*;

    pub trait Instance {
        const REGS: Regs;
    }
}

pub trait Word: word::Word {}

pub trait Instance: Peripheral<P = Self> + sealed::Instance + RccPeripheral {}
pin_trait!(SckAPin, Instance);
pin_trait!(SckBPin, Instance);
pin_trait!(FsAPin, Instance);
pin_trait!(FsBPin, Instance);
pin_trait!(SdAPin, Instance);
pin_trait!(SdBPin, Instance);
pin_trait!(MclkAPin, Instance);
pin_trait!(MclkBPin, Instance);

dma_trait!(DmaA, Instance);
dma_trait!(DmaB, Instance);

foreach_peripheral!(
    (sai, $inst:ident) => {
        impl sealed::Instance for peripherals::$inst {
            const REGS: Regs = crate::pac::$inst;
        }

        impl Instance for peripherals::$inst {}
    };
);

impl<'d, T: Instance> SetConfig for Sai<'d, T> {
    type Config = Config;
    type ConfigError = ();
    fn set_config(&mut self, _config: &Self::Config) -> Result<(), ()> {
        // self.reconfigure(*config);

        Ok(())
    }
}