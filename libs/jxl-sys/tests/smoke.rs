//! 生の FFI だけで単葉を符号化し、libjxl と jxl-rs の両方で読み戻して画素を突き合わせる

use jxl::api::{self, states::Initialized};
use jxl_sys::*;
use std::ffi::{c_int, c_void};
use std::ptr;

const WIDTH: usize = 37;
const HEIGHT: usize = 23;
const CHANNELS: usize = 3;

/// 位置から決まる画素。食い違いが起きた場所が値に現れる
fn source_pixels() -> Vec<u8> {
    let mut pixels = Vec::with_capacity(WIDTH * HEIGHT * CHANNELS);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            pixels.push((x * 7) as u8);
            pixels.push((y * 11) as u8);
            pixels.push((x * y) as u8);
        }
    }
    pixels
}

/// 符号化の各段の戻り値を検める
fn expect_success(enc: *mut JxlEncoder, status: c_int, what: &str) {
    assert_eq!(
        status,
        JXL_ENC_SUCCESS,
        "{what} が失敗した (JxlEncoderError = {})",
        unsafe { JxlEncoderGetError(enc) }
    );
}

/// sRGB・原色空間・可逆で 1 枚だけ符号化する
fn encode_lossless_still(pixels: &[u8]) -> Vec<u8> {
    unsafe {
        let runner = JxlThreadParallelRunnerCreate(ptr::null(), 2);
        assert!(!runner.is_null());

        let enc = JxlEncoderCreate(ptr::null());
        assert!(!enc.is_null());

        expect_success(
            enc,
            JxlEncoderSetParallelRunner(enc, Some(JxlThreadParallelRunner), runner),
            "SetParallelRunner",
        );

        let mut info = std::mem::MaybeUninit::<JxlBasicInfo>::zeroed();
        JxlEncoderInitBasicInfo(info.as_mut_ptr());
        let mut info = info.assume_init();
        info.xsize = WIDTH as u32;
        info.ysize = HEIGHT as u32;
        info.bits_per_sample = 8;
        info.num_color_channels = CHANNELS as u32;
        info.uses_original_profile = JXL_TRUE;
        expect_success(enc, JxlEncoderSetBasicInfo(enc, &info), "SetBasicInfo");

        let mut color = std::mem::MaybeUninit::<JxlColorEncoding>::zeroed();
        JxlColorEncodingSetToSRGB(color.as_mut_ptr(), JXL_FALSE);
        let color = color.assume_init();
        expect_success(
            enc,
            JxlEncoderSetColorEncoding(enc, &color),
            "SetColorEncoding",
        );

        let settings = JxlEncoderFrameSettingsCreate(enc, ptr::null());
        assert!(!settings.is_null());
        expect_success(
            enc,
            JxlEncoderFrameSettingsSetOption(settings, JXL_ENC_FRAME_SETTING_EFFORT, 3),
            "SetOption(EFFORT)",
        );
        // 可逆の指定が distance を上書きする
        expect_success(
            enc,
            JxlEncoderSetFrameDistance(settings, JxlEncoderDistanceFromQuality(90.0)),
            "SetFrameDistance",
        );
        expect_success(
            enc,
            JxlEncoderSetFrameLossless(settings, JXL_TRUE),
            "SetFrameLossless",
        );

        let mut header = std::mem::MaybeUninit::<JxlFrameHeader>::zeroed();
        JxlEncoderInitFrameHeader(header.as_mut_ptr());
        let header = header.assume_init();
        expect_success(
            enc,
            JxlEncoderSetFrameHeader(settings, &header),
            "SetFrameHeader",
        );

        let format = JxlPixelFormat {
            num_channels: CHANNELS as u32,
            data_type: JXL_TYPE_UINT8,
            endianness: JXL_NATIVE_ENDIAN,
            align: 0,
        };
        expect_success(
            enc,
            JxlEncoderAddImageFrame(
                settings,
                &format,
                pixels.as_ptr().cast::<c_void>(),
                pixels.len(),
            ),
            "AddImageFrame",
        );

        JxlEncoderCloseInput(enc);

        let mut output = vec![0u8; 64];
        let mut written = 0usize;
        loop {
            let mut next_out = output.as_mut_ptr().add(written);
            let mut avail_out = output.len() - written;
            let status = JxlEncoderProcessOutput(enc, &mut next_out, &mut avail_out);
            written = output.len() - avail_out;
            match status {
                JXL_ENC_SUCCESS => break,
                JXL_ENC_NEED_MORE_OUTPUT => output.resize(output.len() * 2, 0),
                _ => panic!("ProcessOutput が失敗した (JxlEncoderError = {})", {
                    JxlEncoderGetError(enc)
                }),
            }
        }
        output.truncate(written);

        JxlEncoderDestroy(enc);
        JxlThreadParallelRunnerDestroy(runner);

        output
    }
}

/// 復号の各段の戻り値を検める
fn expect_decoded(status: c_int, what: &str) {
    assert_eq!(status, JXL_DEC_SUCCESS, "{what} が失敗した");
}

/// 購読した事象を観測した記録
#[derive(Default)]
struct SeenEvents {
    basic_info: bool,
    full_image: bool,
}

/// 合成後のキャンバスを丸ごと受け取る
///
/// 入力は 2 つに割って与え、読み残しの繋ぎ目を `JxlDecoderReleaseInput` から取る。
fn decode_still(encoded: &[u8]) -> (Vec<u8>, SeenEvents) {
    unsafe {
        let runner = JxlThreadParallelRunnerCreate(ptr::null(), 2);
        assert!(!runner.is_null());

        let dec = JxlDecoderCreate(ptr::null());
        assert!(!dec.is_null());

        expect_decoded(
            JxlDecoderSetParallelRunner(dec, Some(JxlThreadParallelRunner), runner),
            "SetParallelRunner",
        );
        expect_decoded(JxlDecoderSetCoalescing(dec, JXL_TRUE), "SetCoalescing");
        expect_decoded(
            JxlDecoderSubscribeEvents(dec, JXL_DEC_BASIC_INFO | JXL_DEC_FULL_IMAGE),
            "SubscribeEvents",
        );

        let mut fed = encoded.len() / 2;
        expect_decoded(
            JxlDecoderSetInput(dec, encoded.as_ptr(), fed),
            "SetInput(前半)",
        );

        let format = JxlPixelFormat {
            num_channels: CHANNELS as u32,
            data_type: JXL_TYPE_UINT8,
            endianness: JXL_NATIVE_ENDIAN,
            align: 0,
        };
        let mut pixels = vec![0u8; WIDTH * HEIGHT * CHANNELS];
        let mut seen = SeenEvents::default();

        loop {
            match JxlDecoderProcessInput(dec) {
                JXL_DEC_BASIC_INFO => {
                    assert!(!seen.basic_info, "BASIC_INFO が二度返った");
                    seen.basic_info = true;
                }
                JXL_DEC_NEED_MORE_INPUT => {
                    assert!(fed < encoded.len(), "入力を使い切っても足りない");
                    let from = fed - JxlDecoderReleaseInput(dec);
                    expect_decoded(
                        JxlDecoderSetInput(dec, encoded.as_ptr().add(from), encoded.len() - from),
                        "SetInput(後半)",
                    );
                    fed = encoded.len();
                }
                JXL_DEC_NEED_IMAGE_OUT_BUFFER => {
                    let mut size = 0usize;
                    expect_decoded(
                        JxlDecoderImageOutBufferSize(dec, &format, &mut size),
                        "ImageOutBufferSize",
                    );
                    assert_eq!(size, WIDTH * HEIGHT * CHANNELS);
                    expect_decoded(
                        JxlDecoderSetImageOutBuffer(
                            dec,
                            &format,
                            pixels.as_mut_ptr().cast::<c_void>(),
                            pixels.len(),
                        ),
                        "SetImageOutBuffer",
                    );
                }
                JXL_DEC_FULL_IMAGE => {
                    assert!(!seen.full_image, "FULL_IMAGE が二度返った");
                    seen.full_image = true;
                }
                JXL_DEC_SUCCESS => break,
                status => panic!("ProcessInput が {status} を返した"),
            }
        }

        assert_eq!(fed, encoded.len(), "前半だけで復号が終わった");
        assert_eq!(JxlDecoderReleaseInput(dec), 0, "読み残しがある");

        JxlDecoderDestroy(dec);
        JxlThreadParallelRunnerDestroy(runner);

        (pixels, seen)
    }
}

/// 段を1つ進める。入力を使い切らずに止まったら符号化が不完全
fn complete<T, U>(result: api::ProcessingResult<T, U>, what: &str) -> T {
    match result {
        api::ProcessingResult::Complete { result } => result,
        api::ProcessingResult::NeedsMoreInput { size_hint, .. } => {
            panic!("{what} が入力不足で止まった (あと {size_hint} バイト)")
        }
    }
}

#[test]
fn a_lossless_still_round_trips_byte_for_byte() {
    let source = source_pixels();
    let encoded = encode_lossless_still(&source);
    assert!(!encoded.is_empty());

    let mut input: &[u8] = &encoded;
    let decoder = api::JxlDecoder::<Initialized>::new(api::JxlDecoderOptions::default());
    let mut decoder = complete(
        decoder.process(&mut input, None).unwrap(),
        "画像情報の読み出し",
    );

    let info = decoder.basic_info();
    assert_eq!(info.size, (WIDTH, HEIGHT));
    assert!(info.animation.is_none());
    assert!(info.uses_original_profile);
    assert!(info.extra_channels.is_empty());

    decoder.set_pixel_format(api::JxlPixelFormat::rgb8(0));
    let decoder = complete(
        decoder.process(&mut input, None).unwrap(),
        "フレーム情報の読み出し",
    );

    let mut decoded = vec![0u8; WIDTH * HEIGHT * CHANNELS];
    let decoder = {
        let mut buffers = [api::JxlOutputBuffer::new(
            &mut decoded,
            HEIGHT,
            WIDTH * CHANNELS,
        )];
        complete(
            decoder.process(&mut input, &mut buffers, None).unwrap(),
            "画素の読み出し",
        )
    };

    assert_eq!(decoded, source);

    let frames = decoder.scanned_frames();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].is_last);
}

#[test]
fn the_bundled_decoder_returns_the_subscribed_events_and_the_pixels() {
    let source = source_pixels();
    let encoded = encode_lossless_still(&source);
    assert!(!encoded.is_empty());

    let (decoded, seen) = decode_still(&encoded);

    assert!(seen.basic_info, "BASIC_INFO が返らなかった");
    assert!(seen.full_image, "FULL_IMAGE が返らなかった");
    assert_eq!(decoded, source);
}
