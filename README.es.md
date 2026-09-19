# Omnifrons

**Un contexto. Cualquier modelo.**

> Esta página es solo la entrada al proyecto. La versión canónica es
> [`README.md`](README.md), en inglés, igual que el resto de la documentación,
> las especificaciones y la evidencia. Si alguna vez las dos difieren, la que
> vale es la inglesa.

Omnifrons es una fachada de escritorio, todavía en diseño, para los *harnesses*
de agentes de línea de comandos que cada persona instala por su cuenta. La idea
es tener una identidad de agente duradera, un espacio de trabajo y una
superficie de control única, mientras el modelo o el harness por debajo puede
cambiar.

## En qué estado está, sin adornos

Es un *spike* en modo desarrollo. Existen un workspace de Rust y una cáscara
Tauri: supervisión de procesos con IPC tipado, aprobación de ejecutables,
adaptadores de línea y de pseudoterminal, y superficies locales de publicación y
recuperación. **No hay release ni instalador**, y no hay fecha.

Las secciones de continuidad y almacenamiento del README en inglés describen a
dónde va el diseño, no lo que hay hecho. Allí están marcadas como tales.

## Qué está probado, y qué no

La verificación formal contra el plan VP-001 ya empezó, sobre una imagen de
runner Linux fijada. Lo que se ha observado está archivado en
[`docs/evidence/VP-001/`](docs/evidence/VP-001/), incluidos los fallos:

- un escenario que ni siquiera llegó a lanzar, registrado como `uncertain`;
- un proceso descendiente que sobrevivió a una parada que el producto había
  dado por limpia, registrado como `fail`;
- y el mismo escenario en verde, `pass`, contra el arreglo que ese fallo
  provocó.

El registro es de solo añadir: un resultado equivocado se corrige con un
registro nuevo, nunca editando el anterior. Por eso los fallos siguen ahí.

## Por dónde entrar

- **Compilar y probar:** [`docs/repository-layout.md`](docs/repository-layout.md),
  sección *Build and test commands*. En corto, `cargo test --workspace` para la
  parte Rust y `pnpm test` dentro de `renderer/`.
- **Qué está especificado:** [`openspec/specs/`](openspec/specs/).
- **Qué está probado:** [`docs/evidence/VP-001/`](docs/evidence/VP-001/).
- **Qué se construyó, trozo a trozo:** [`docs/spike-log.md`](docs/spike-log.md).
- **Índice de documentación:** [`docs/README.md`](docs/README.md).

## Licencia

[Apache License 2.0](LICENSE). Los productos y marcas de terceros se rigen por
sus propios términos. Omnifrons no reclama respaldo alguno de los proyectos ni
de los proveedores mencionados en su diseño.
