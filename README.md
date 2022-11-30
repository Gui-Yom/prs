# tp3rs

UDP control protocol unidirectional server implementation.

## Building and running

Cargo and a rust toolchain must be installed.

### Running

```shell
git clone https://github.com/Gui-Yom/prs
cd prs
cargo run
```

### Graphs and data

The application can automatically generate graphs thanks to plotly.
Compile with the `trace` feature to capture trace data and generate graphs.

```shell
cargo run --features trace
```

### Optimized release build

For a release, build the program with optimizations and remove trace calls at compile time :

```shell
cargo run --release --features tracing/release_max_level_debug
```

## Reverse engineered packet drop function

```c
/* Retourne vrai si le message doit être perdu */
bool drop_message(double debitMo,double *seuil,int seq) {
  int tirage;
  double dropRate;
  
                    /* Recalcul du débit limite tous les 100 paquets */
  if (seq % 100 == 0) {
    tirage = rand();
    *seuil = ((double)(tirage % 100) / 100.0) * 20.0;
  }
                    /* Perte à 1% */
  dropRate = 0.01;
                    /* si le débit actuel est supérieur au seuil, le drop rate devient 5% */
  if (*seuil <= debitMo) {
    dropRate = 0.05;
  }
                    /* Tirage de la perte du paquet */
  tirage = rand();
  return (double)(tirage % 100) / 100.0 < dropRate;
}
```

## Current performances

Best throughput : 11.1 MB/s
