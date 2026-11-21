package io.github.cristianencarnacion.ferroreport;

/** A report could not be rendered: the message says why, in the engine's words. */
public class FerroreportException extends RuntimeException {
    private static final long serialVersionUID = 1L;

    public FerroreportException(String message) {
        super(message);
    }

    public FerroreportException(String message, Throwable cause) {
        super(message, cause);
    }
}
