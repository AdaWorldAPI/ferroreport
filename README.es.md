# ferroreport

**Tus plantillas de JasperReports, sin JVM.** ferroreport lee archivos `.jrxml` y los convierte a
PDF desde un único binario nativo: más rápido, con una fracción de la memoria y seguro para
plantillas que no escribiste tú.

> Estado: **vista previa (fase 2).** Todas las plantillas comparables de la batería de pruebas
> salen píxel por píxel igual que en JasperReports, incluyendo bandas divididas entre páginas,
> imágenes, subreportes y códigos QR. Todavía faltan algunas funciones; ver
> [compatibilidad](README.md#compatibility). No está afiliado a Jaspersoft ni a Cloud Software Group.

## Por qué

- **Sin JVM.** Un binario de ~8 MB (con las fuentes incluidas) que arranca en milisegundos.
  Importa en CLIs, funciones serverless y contenedores.
- **Salida idéntica.** Cada fixture se renderiza con los dos motores y se compara píxel por
  píxel: hoy los 17 comparables salen idénticos.
- **Seguro con plantillas ajenas.** JasperReports compila las expresiones a Java y carga
  archivos `.jasper` serializados; así ocurrieron fallos recientes de ejecución remota de código
  (CVE-2025-10492, CVE-2026-6009). ferroreport nunca deserializa objetos e interpreta las
  expresiones con una lista cerrada de operaciones: sin reflexión, sin carga de clases, sin
  acceso a la red. Las imágenes y subreportes solo se leen de las carpetas de recursos de la
  plantilla.
- **Úsalo desde cualquier lenguaje.** Paquetes para JavaScript (Node.js, Deno, Bun y
  navegadores) y Python, librería de Rust, CLI, y una API HTTP en imagen Docker para todo lo
  demás (Java, C#, PHP, Go, Ruby...).

## Desde tu lenguaje

- **JavaScript/TypeScript:** `npm install ferroreport` y
  `render(plantilla, filas, parametros, { locale: "es_DO" })` devuelve el PDF.
- **Python:** `pip install ferroreport` y
  `ferroreport.render("factura.jrxml", filas, parametros, locale="es_DO")`.
- **Cualquier lenguaje por HTTP:**
  `docker run -p 8080:8080 -v "$PWD/plantillas:/templates" -e FERROREPORT_TEMPLATES=/templates ghcr.io/cristianencarnacion/ferroreport`
  y luego `POST /render/factura` con `{"data": [...], "params": {...}}` te devuelve el PDF.
- **Rust:** `cargo add ferroreport`. **Línea de comandos:** binarios en
  [Releases](https://github.com/cristianEncarnacion/ferroreport/releases); `-d -` y `-o -` leen
  las filas de stdin y escriben el PDF en stdout.

## Inicio rápido

```bash
cargo install --path crates/ferroreport-cli

ferroreport render factura.jrxml --data filas.json --params parametros.json -o factura.pdf --locale es_DO
ferroreport doctor factura.jrxml     # qué soporta (y qué no) esta plantilla
```

`filas.json` es un arreglo JSON de objetos cuyas claves coinciden con los `<field>` de la
plantilla. Los valores se convierten a la clase que declara cada campo (`BigDecimal`, fechas
ISO-8601 a `java.util.Date`, arreglos anidados a `java.util.List`, base64 a `byte[]`, etc.).

Las imágenes y subreportes se buscan junto a la plantilla (agrega más carpetas con
`--resources CARPETA`). Un subreporte que apunta a `lineas.jasper` se renderiza desde
`lineas.jrxml`: nunca se cargan archivos `.jasper` compilados. Los arreglos JSON anidados
alimentan subreportes, por ejemplo `new JRMapCollectionDataSource($F{lineas})` o
`((JsonDataSource)$P{REPORT_DATA_SOURCE}).subDataSource("lineas")`.

## Qué soporta

- Bandas completas, incluida la división de bandas y frames entre páginas (`splitType`).
- Texto estático y dinámico, líneas, rectángulos, elipses, frames, imágenes PNG/JPEG,
  subreportes, códigos QR y Code 128 (componentes barcode4j).
- Estilos con herencia y estilos condicionales, texto justificado, `isRemoveLineWhenBlank`,
  `isPrintRepeatedValues`, grupos, variables y tiempos de evaluación.
- Los QR se codifican igual que ZXing (misma versión, máscara y módulos) y se dibujan como los
  dibuja JasperReports: sirven para facturas electrónicas impresas, como el sello del e-CF
  dominicano.
- Aún no: gráficos, crosstabs, tablas, texto con markup HTML, multicolumna, conexiones JDBC.
  La tabla completa está en el [README en inglés](README.md#compatibility).

## Rendimiento

Render completo por línea de comandos (desde que arranca el proceso hasta el PDF en disco),
mediana de 5 corridas en una VM Linux de 2 vCPU:

| Fixture | JasperReports 6.20 | ferroreport | Más rápido | Memoria pico |
|---|---|---|---|---|
| 02-invoice (2 páginas) | 3.01 s | 0.010 s | 309× | 194 MiB → 16 MiB |
| 03-grouped-sales (4 páginas) | 3.22 s | 0.013 s | 253× | 205 MiB → 16 MiB |
| 09-large (20,000 filas, 477 páginas) | 7.22 s | 0.39 s | 19× | 489 MiB → 52 MiB |

En reportes pequeños, la mayor parte del tiempo de Jasper es el arranque de la JVM y la
compilación del JRXML, que un servidor de larga vida amortiza. Contando solo sus fases de
llenado y exportación, ferroreport es unas 85× más rápido en los reportes pequeños y 13× en el
grande. Además sus PDF pesan menos (1.1 MB en vez de 4.5 MB en 09-large).

## Roadmap

- **Fase 2 (en curso):** listo: división de bandas, imágenes, subreportes, estilos
  condicionales, texto justificado, códigos QR y Code 128. Siguiente: texto con formato
  (markup), multicolumna, más tipos de códigos de barras y memoria constante en reportes enormes.
- **Fase 3:** paquetes para npm y PyPI, versión WebAssembly con playground en el navegador y
  vista previa en vivo para VS Code.
- **Fase 4:** gráficos, tablas, crosstabs y exportación a Excel.

La documentación completa (compatibilidad, cómo se verifica la salida, cómo contribuir) está en
el [README en inglés](README.md).

## Licencia

MIT o Apache-2.0, a tu elección. Las fuentes DejaVu incluidas tienen su propia licencia.
JasperReports y Jaspersoft son marcas de Cloud Software Group, Inc.; este proyecto es
independiente.
