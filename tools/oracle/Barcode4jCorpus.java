// Prints the SVG barcode4j 2.1 generates for one barcode, configured the way JasperReports
// configures its beans. Used to build crates/ferroreport/tests/data/barcode4j.txt:
//
//   javac -cp "$JASPER_LIB/*" -d /tmp/b4j tools/oracle/Barcode4jCorpus.java
//   while read -r c; do echo "@ $c"; java -cp "/tmp/b4j:$JASPER_LIB/*" Barcode4jCorpus $c \
//     | grep -E "<svg|<rect|<text" | sed -E 's/ xmlns="[^"]*"//; s/font-family="Helvetica" //; s/text-anchor="middle" //'
//   done < tools/oracle/barcode4j-cases.txt > crates/ferroreport/tests/data/barcode4j.txt
//
// Arguments: type message heightPt [moduleWidthPt|-] [textPosition|-] [fontSizePt|-] [key=value...]
import org.krysalis.barcode4j.impl.*;
import org.krysalis.barcode4j.impl.code39.*;
import org.krysalis.barcode4j.impl.upcean.*;
import org.krysalis.barcode4j.impl.int2of5.*;
import org.krysalis.barcode4j.impl.codabar.*;
import org.krysalis.barcode4j.impl.code128.*;
import org.krysalis.barcode4j.impl.datamatrix.*;
import org.krysalis.barcode4j.impl.postnet.*;
import org.krysalis.barcode4j.impl.fourstate.*;
import org.krysalis.barcode4j.output.svg.*;
import org.krysalis.barcode4j.tools.UnitConv;
import org.krysalis.barcode4j.*;
public class Barcode4jCorpus {
  public static void main(String[] a) throws Exception {
    // args: type msg heightPt [moduleWidthPt|-] [textPos|-] [fontSizePt|-] [extra k=v...]
    AbstractBarcodeBean b;
    switch (a[0]) {
      case "code39": b = new Code39Bean(); break;
      case "ean13": b = new EAN13Bean(); break;
      case "ean8": b = new EAN8Bean(); break;
      case "upca": b = new UPCABean(); break;
      case "upce": b = new UPCEBean(); break;
      case "i25": b = new Interleaved2Of5Bean(); break;
      case "codabar": b = new CodabarBean(); break;
      case "ean128": b = new EAN128Bean(); break;
      case "datamatrix": b = new DataMatrixBean(); break;
      case "postnet": b = new POSTNETBean(); break;
      case "royalmail": b = new RoyalMailCBCBean(); break;
      case "usps": b = new USPSIntelligentMailBean(); break;
      default: b = new Code128Bean();
    }
    b.setFontName("Helvetica");
    double fs = a.length > 5 && !a[5].equals("-") ? Double.parseDouble(a[5]) : 10;
    b.setFontSize(UnitConv.pt2mm(fs));
    if (a.length > 3 && !a[3].equals("-")) b.setModuleWidth(UnitConv.pt2mm(Double.parseDouble(a[3])));
    if (a.length > 4 && !a[4].equals("-")) b.setMsgPosition(HumanReadablePlacement.byName(a[4]));
    for (int i = 6; i < a.length; i++) {
      String[] kv = a[i].split("=");
      switch (kv[0]) {
        case "checksum":
          if (b instanceof Code39Bean) ((Code39Bean)b).setChecksumMode(ChecksumMode.byName(kv[1]));
          if (b instanceof Interleaved2Of5Bean) ((Interleaved2Of5Bean)b).setChecksumMode(ChecksumMode.byName(kv[1]));
          if (b instanceof UPCEANBean) ((UPCEANBean)b).setChecksumMode(ChecksumMode.byName(kv[1]));
          if (b instanceof EAN128Bean) ((EAN128Bean)b).setChecksumMode(ChecksumMode.byName(kv[1]));
          if (b instanceof POSTNETBean) ((POSTNETBean)b).setChecksumMode(ChecksumMode.byName(kv[1]));
          if (b instanceof AbstractFourStateBean) ((AbstractFourStateBean)b).setChecksumMode(ChecksumMode.byName(kv[1]));
          break;
        case "wide":
          if (b instanceof Code39Bean) ((Code39Bean)b).setWideFactor(Double.parseDouble(kv[1]));
          if (b instanceof Interleaved2Of5Bean) ((Interleaved2Of5Bean)b).setWideFactor(Double.parseDouble(kv[1]));
          if (b instanceof CodabarBean) ((CodabarBean)b).setWideFactor(Double.parseDouble(kv[1]));
          break;
        case "startstop": ((Code39Bean)b).setDisplayStartStop(Boolean.parseBoolean(kv[1])); break;
        case "dispcheck":
          if (b instanceof Code39Bean) ((Code39Bean)b).setDisplayChecksum(Boolean.parseBoolean(kv[1]));
          else ((Interleaved2Of5Bean)b).setDisplayChecksum(Boolean.parseBoolean(kv[1])); break;
        case "ext": ((Code39Bean)b).setExtendedCharSetEnabled(Boolean.parseBoolean(kv[1])); break;
        case "gap": ((Code39Bean)b).setIntercharGapWidth(Double.parseDouble(kv[1])); break;
        case "shape": ((DataMatrixBean)b).setShape(SymbolShapeHint.byName(kv[1])); break;
        case "short": ((POSTNETBean)b).setShortBarHeight(UnitConv.pt2mm(Double.parseDouble(kv[1]))); break;
        case "pchecksum": ((POSTNETBean)b).setChecksumMode(ChecksumMode.byName(kv[1])); break;
        case "ascender": ((AbstractFourStateBean)b).setAscenderHeight(UnitConv.pt2mm(Double.parseDouble(kv[1]))); break;
        case "track": ((AbstractFourStateBean)b).setTrackHeight(UnitConv.pt2mm(Double.parseDouble(kv[1]))); break;
        case "fgap": ((AbstractFourStateBean)b).setIntercharGapWidth(UnitConv.pt2mm(Double.parseDouble(kv[1]))); break;
        case "echecksum": ((EAN128Bean)b).setChecksumMode(ChecksumMode.byName(kv[1])); break;
        case "vquiet": b.setVerticalQuietZone(UnitConv.pt2mm(Double.parseDouble(kv[1]))); break;
        case "quiet": b.doQuietZone(true); b.setQuietZone(UnitConv.pt2mm(Double.parseDouble(kv[1]))); break;
      }
    }
    b.setHeight(UnitConv.pt2mm(Double.parseDouble(a[2])));
    SVGCanvasProvider p = new SVGCanvasProvider(false, 0);
    b.generateBarcode(p, a[1]);
    javax.xml.transform.Transformer t = javax.xml.transform.TransformerFactory.newInstance().newTransformer();
    t.setOutputProperty("omit-xml-declaration", "yes");
    java.io.StringWriter w = new java.io.StringWriter();
    t.transform(new javax.xml.transform.dom.DOMSource(p.getDOM()), new javax.xml.transform.stream.StreamResult(w));
    System.out.println(w.toString().replace("><", ">\n<"));
  }
}
