//! EDID decoding: vendor, model, serial and physical size from the base block.

use super::geometry::Size;

const HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
const DESCRIPTORS: [usize; 4] = [54, 72, 90, 108];
const TAG_SERIAL: u8 = 0xFF;
const TAG_TEXT: u8 = 0xFE;
const TAG_NAME: u8 = 0xFC;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edid {
    /// Three-letter PNP manufacturer ID, e.g. `AUO`.
    pub pnp: String,
    pub product: u16,
    pub serial: u32,
    /// The 0xFC display name descriptor.
    pub name: Option<String>,
    /// The 0xFF serial string descriptor.
    pub serial_text: Option<String>,
    /// Every 0xFE unspecified-text descriptor, in order.
    pub texts: Vec<String>,
    /// Image size from the first detailed timing descriptor.
    pub size_mm: Option<Size>,
}

impl Edid {
    /// Decodes the 128-byte base block; extension blocks are ignored.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 128 || bytes[..8] != HEADER {
            return None;
        }
        let id = u16::from_be_bytes([bytes[8], bytes[9]]);
        let letter = |shift: u16| char::from(b'A' - 1 + ((id >> shift) & 0x1F) as u8);
        let pnp: String = [letter(10), letter(5), letter(0)].into_iter().collect();
        let product = u16::from_le_bytes([bytes[10], bytes[11]]);
        let serial = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);

        let mut edid = Edid {
            pnp,
            product,
            serial,
            name: None,
            serial_text: None,
            texts: Vec::new(),
            size_mm: None,
        };
        for &at in &DESCRIPTORS {
            let d = &bytes[at..at + 18];
            // A display descriptor starts with three zero bytes; anything else is a timing.
            if d[..3] != [0, 0, 0] {
                if at == DESCRIPTORS[0] {
                    edid.size_mm = timing_size(d);
                }
                continue;
            }
            let text = descriptor_text(&d[5..]);
            match d[3] {
                TAG_NAME if !text.is_empty() => edid.name = Some(text),
                TAG_SERIAL if !text.is_empty() => edid.serial_text = Some(text),
                TAG_TEXT if !text.is_empty() => edid.texts.push(text),
                _ => {}
            }
        }
        if edid.size_mm.is_none() && bytes[21] > 0 && bytes[22] > 0 {
            // Fall back to the basic display parameters, which are in centimetres.
            edid.size_mm = Some(Size::new(
                i32::from(bytes[21]) * 10,
                i32::from(bytes[22]) * 10,
            ));
        }
        Some(edid)
    }

    /// The vendor name for well-known PNP IDs.
    pub fn vendor(&self) -> Option<&'static str> {
        vendor_name(&self.pnp)
    }

    /// The model: the 0xFC name, else the last 0xFE text (panels put the part number last),
    /// else the PNP ID and product code.
    pub fn model(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        if let Some(text) = self.texts.last() {
            return text.clone();
        }
        format!("{} {:04X}", self.pnp, self.product)
    }

    /// The model, prefixed with the vendor unless the model already names it:
    /// `Philips FTV`, `AUO B156HAN12.H`, `LG Display LP156WFC-SPD1`.
    pub fn display_name(&self) -> String {
        let model = self.model();
        let lower = model.to_lowercase();
        match self.vendor() {
            Some(vendor)
                if !lower.contains(&vendor.to_lowercase())
                    && !lower.contains(&self.pnp.to_lowercase()) =>
            {
                format!("{vendor} {model}")
            }
            _ => model,
        }
    }

    /// The serial string descriptor, else the numeric serial when it is set.
    pub fn serial_string(&self) -> Option<String> {
        self.serial_text
            .clone()
            .or_else(|| (self.serial != 0).then(|| self.serial.to_string()))
    }

    /// What the display says it is, in the form both backends share.
    pub fn identity(&self) -> Identity {
        Identity {
            make: Some(
                self.vendor()
                    .map_or_else(|| self.pnp.clone(), str::to_owned),
            ),
            model: Some(self.model()),
            serial: self.serial_string(),
            label: self.display_name(),
        }
    }
}

/// Who made a display and which one it is. On X11 it comes from the EDID; a Wayland compositor
/// reports it directly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    /// The name the panels show: `Philips FTV`, `AUO B156HAN12.H`.
    pub label: String,
}

/// Image size in mm from a detailed timing descriptor: bytes 12 and 13 hold the low eight bits,
/// byte 14 the high nibbles.
fn timing_size(d: &[u8]) -> Option<Size> {
    let w = i32::from(d[12]) | (i32::from(d[14] >> 4) << 8);
    let h = i32::from(d[13]) | (i32::from(d[14] & 0x0F) << 8);
    (w > 0 && h > 0).then_some(Size::new(w, h))
}

/// Descriptor text ends at 0x0A and is padded with spaces.
fn descriptor_text(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0x0A).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).trim().to_owned()
}

fn vendor_name(pnp: &str) -> Option<&'static str> {
    Some(match pnp {
        "ACR" => "Acer",
        "AOC" => "AOC",
        "AUO" => "AUO",
        "AUS" | "ASU" => "ASUS",
        "BNQ" => "BenQ",
        "BOE" => "BOE",
        "CMN" => "Innolux",
        "DEL" => "Dell",
        "ENC" => "EIZO",
        "GSM" => "LG",
        "HWP" => "HP",
        "LEN" => "Lenovo",
        "LGD" => "LG Display",
        "MSI" => "MSI",
        "PHL" => "Philips",
        "SAM" => "Samsung",
        "SDC" => "Samsung Display",
        "SHP" => "Sharp",
        "VSC" => "ViewSonic",
        _ => return None,
    })
}

/// Decodes the hex dump xrandr prints under `EDID:`.
pub fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = hex.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUO: &str = "00ffffffffffff0006af955800000000241e0104b522137803ee95a3544c99260f505400000001010101010101010101\
                       010101010101349e80a07038644010103e0058c1100000180000000f0000000000000000000000000020000000fd003c\
                       a5c3c329010a202020202020000000fe004231353648414e31322e48200a0056";

    #[test]
    fn decodes_the_laptop_panel() {
        let edid = Edid::parse(&decode_hex(AUO).unwrap()).unwrap();
        assert_eq!(edid.pnp, "AUO");
        assert_eq!(edid.product, 0x5895);
        assert_eq!(edid.serial, 0);
        assert_eq!(edid.name, None);
        assert_eq!(edid.texts, vec!["B156HAN12.H".to_owned()]);
        assert_eq!(edid.size_mm, Some(Size::new(344, 193)));
        assert_eq!(edid.model(), "B156HAN12.H");
        assert_eq!(edid.display_name(), "AUO B156HAN12.H");
        assert_eq!(edid.serial_string(), None);
        assert_eq!(
            edid.identity(),
            Identity {
                make: Some("AUO".to_owned()),
                model: Some("B156HAN12.H".to_owned()),
                serial: None,
                label: "AUO B156HAN12.H".to_owned(),
            }
        );
    }

    #[test]
    fn text_descriptors_need_three_zero_bytes() {
        let mut bytes = decode_hex(AUO).unwrap();
        // Turn the 0xFE descriptor at 108 into something that only looks like text from byte 3.
        bytes[108] = 0x01;
        let edid = Edid::parse(&bytes).unwrap();
        assert!(edid.texts.is_empty());
        assert_eq!(edid.model(), "AUO 5895");
    }

    #[test]
    fn rejects_a_short_or_headerless_block() {
        assert_eq!(Edid::parse(&[0; 64]), None);
        assert_eq!(Edid::parse(&[0; 128]), None);
    }

    #[test]
    fn decodes_hex_with_whitespace() {
        assert_eq!(decode_hex("00ff\n\t 10"), Some(vec![0x00, 0xFF, 0x10]));
        assert_eq!(decode_hex("0"), None);
        assert_eq!(decode_hex("zz"), None);
    }
}
