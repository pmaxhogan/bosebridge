# BMAP over Bluetooth serial, as used by bosebridge

BMAP is the protocol the Bose app speaks to Bose headphones. These notes cover
the small part bosebridge uses. They were derived from the Bose Android app
(10.2.4) and confirmed against QuietComfort Ultra headphones (2nd gen) on
2026-09-11. Nothing from the app is reproduced here.

## Transport

- Standard Bluetooth Serial Port Profile, UUID `00001101-0000-1000-8000-00805F9B34FB`.
- Windows registers it as a "Standard Serial over Bluetooth link (COMx)" the
  moment the headphones are paired. The COM number lives in the registry under
  `HKLM\SYSTEM\CurrentControlSet\Enum\BTHENUM\{00001101-...}_VID&0001009E_PID&xxxx\...<mac>...\Device Parameters\PortName`.
  `0001009E` is Bose's vendor id.
- No authentication or handshake: plain requests are answered immediately.
  (The protocol has an Authentication function block, but Device Management
  does not require it.)
- Opening the COM port makes Windows bring up the Bluetooth link if it is
  down, which pages the headphones.

## Framing

Four-byte header followed by the payload:

```
byte 0  function block
byte 1  function
byte 2  (deviceId << 6) | (port << 4) | operator      (deviceId and port are 0 in practice)
byte 3  payload length
```

Operators: `SET=0 GET=1 SET_GET=2 STATUS=3 ERROR=4 START=5 RESULT=6 PROCESSING=7`.
Replies to a GET use STATUS; a START gets PROCESSING then RESULT. An ERROR
reply's first payload byte is the code (`11` device not found, `12` busy,
`10` invalid state, `20` insecure transport, and so on).

Function blocks: `PRODUCT_INFO=0 SETTINGS=1 STATUS=2 FIRMWARE_UPDATE=3
DEVICE_MANAGEMENT=4 AUDIO_MANAGEMENT=5 CALL_MANAGEMENT=6 CONTROL=7 DEBUG=8
NOTIFICATION=9 HEARING_ASSISTANCE=12 DATA_COLLECTION=13 HEART_RATE=14
PEER_BUD=15 VPA=16 WIFI=17 AUTHENTICATION=18 CLOUD=20 AUGMENTED_REALITY=21
AUDIO_MODES=31`.

## Device Management (block 4)

Functions: `FUNCTION_BLOCK_INFO=0 CONNECT=1 DISCONNECT=2 REMOVE_DEVICE=3
LIST_DEVICES=4 INFO=5 EXTENDED_INFO=6 CLEAR_DEVICE_LIST=7 PAIRING_MODE=8
APP_ADDRESS=9 PREPARE_P2P=10 P2P_MODE=11 ROUTING=12 CONNECTION_PRIORITY=16
AVAILABLE_TO_CONNECT=18`.

| request | bytes | reply |
|---|---|---|
| list devices | `04 04 01 00` | STATUS: one lead byte, then 6-byte MACs |
| info | `04 05 01 06 <mac>` | STATUS: `<mac> <flags> <b7> <b8> <name...>` |
| connect | `04 01 05 07 00 <mac>` | PROCESSING `<mac>`, then RESULT `<mac> 0f 00` |
| disconnect | `04 02 05 06 <mac>` | same shape (no leading flag byte; with one the headphones answer InvalidData) |

`flags` bits: 0 connected, 1 the device that asked, 2 another Bose product
(for those, bytes 7..8 are a product id, byte 9 a variant, and the name starts
at byte 10). Bytes 7 and 8 for ordinary devices were `02 03` for both a Pixel 8
and a Windows PC; their meaning is not decoded. The two bytes after the MAC in
the CONNECT result (`0f 00`) are ignored by the Bose app.

"Connected" in the flags byte means any link, including the serial link that
carried the question, so the querying PC always reads as connected.

## Captured session

Headphones `68:F2:1F:37:02:82`, PC `C8:94:02:70:6E:56`, phone `94:45:60:2F:CC:9B`:

```
> 00 00 01 00                       < 00 00 03 05 31 2e 31 2e 30            "1.1.0"
> 00 01 01 00                       < 00 01 03 05 31 2e 32 2e 30            BMAP "1.2.0"
> 04 00 01 00                       < 04 00 03 05 31 2e 31 2e 30            "1.1.0"
> 04 04 01 00                       < 04 04 03 0d 03 94 45 60 2f cc 9b c8 94 02 70 6e 56
> 04 05 01 06 94 45 60 2f cc 9b     < 04 05 03 10 94 45 60 2f cc 9b 01 02 03 "Pixel 8"
> 04 05 01 06 c8 94 02 70 6e 56     < 04 05 03 10 c8 94 02 70 6e 56 03 02 03 "MAX-PRO"
> 04 01 05 07 00 c8 94 02 70 6e 56  < 04 01 07 06 c8 94 02 70 6e 56
                                    < 04 01 06 08 c8 94 02 70 6e 56 0f 00
```

These bytes are the fixtures for the `bmap` crate's tests.

## Related

The same headphones also expose the Spotify Tap RFCOMM service
(`9B26D8C0-A8ED-440B-95B0-C4714A518BCC`), which Windows lists too. That path is
what [tapshim](https://github.com/pmaxhogan/tapshim) uses on Android.
