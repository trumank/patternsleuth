# patternsleuth
A test suite for finding robust patterns used to locate common functions and globals in Unreal Engine games. For use with [UE4SS](https://github.com/UE4SS-RE/RE-UE4SS).

## usage
Drop the game executables into the `games` directory:

```bash
$ tree games
games
├── AstroColony
│   └── AstroColony-Win64-Shipping.exe
├── FSD
│   └── FSD-Win64-Shipping.exe
└── TwinStick
    └── TwinStick-Win64-Shipping.exe
```

Run tests

```bash
$ cargo run --release
```

![tests](https://github.com/trumank/patternsleuth/assets/1144160/0591093c-ea8d-4201-998c-8c6eb4a7fdff)

## acknowledgements
Thanks to,
- [LongerWarrior](https://github.com/LongerWarrior) - for providing a truly massive collection of games to test against as well as finding many very reliable patterns and providing lots of assistance with reversing of more unusual games
- [Narknon](https://github.com/Narknon) - for providing many games as well as clean source builds with symbols
- [FransBouma](https://github.com/FransBouma) - for seeding the project with [UUU](https://opm.fransbouma.com/uuuv5.htm) patterns and many game dumps
- [praydog](https://github.com/praydog) - for inspiration and ideas behind string based symbol resolvers, similar to those used in [UEVR](https://github.com/praydog/UEVR)
