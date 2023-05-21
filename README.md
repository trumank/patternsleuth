# patternsleuth
A test suite for finding robust patterns used to locate common functions and globals in Unreal Engine games. For use with [UE4SS](https://github.com/UE4SS-RE/RE-UE4SS).

## usage
Drop the game executable and a UE4SS.log from a successful hook into the `games` directory:

```bash
$ tree games
games
├── AstroColony
│   ├── AstroColony-Win64-Shipping.exe
│   └── UE4SS.log
├── FSD
│   ├── FSD-Win64-Shipping.exe
│   └── UE4SS.log
└── TwinStick
    ├── TwinStick-Win64-Shipping.exe
    └── UE4SS.log
```

Run tests

```bash
$ cargo run --release
```

![tests](https://github.com/trumank/patternsleuth/assets/1144160/0591093c-ea8d-4201-998c-8c6eb4a7fdff)