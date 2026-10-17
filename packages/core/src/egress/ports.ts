/**
 * Egress ports: what bringing the proxy up needs from the outside world.
 * Implemented by an adapter in @sdw/infra; replaced by fakes in tests.
 */
export interface EgressPorts {
  /** Launch the proxy process. May throw (binary missing); the use case carries on. */
  startProxy(): Promise<void>;
  /** Something accepts connections on the proxy port. */
  proxyListening(): Promise<boolean>;
  /** Nodes the subscription provider has loaded; null when the controller does not answer. */
  providerNodes(): Promise<number | null>;
  /** GLOBAL defaults to DIRECT, which bypasses the proxy: point it at the auto-select group. */
  selectAutoNode(): Promise<void>;
  /** One request to the probe target THROUGH the proxy. */
  probe(): Promise<boolean>;
  sleep(ms: number): Promise<void>;
  /** ISO timestamp. */
  now(): string;
  log(message: string): void;
}
