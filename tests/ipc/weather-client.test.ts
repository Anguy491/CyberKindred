import { describe, expect, it } from "vitest";

import { CyberKindredIpcClient, IpcInvocationError, type IpcTransport } from "../../src/ipc";

const REQUEST_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a210";
const CANDIDATE_ID = "018f47c0-8b8b-7c35-8bf7-278e15b1a211";

class Transport implements IpcTransport {
  readonly calls: Array<{ command: string; args: Readonly<Record<string, unknown>> }> = [];
  response: unknown;
  async invoke<Response>(command: string, args: Readonly<Record<string, unknown>>): Promise<Response> {
    this.calls.push({ command, args });
    return this.response as Response;
  }
  async listen(): Promise<() => void> { return () => undefined; }
}

describe("[TASK-023] weather IPC", () => {
  it("uses the strict API-048/API-049 request envelopes and validates responses", async () => {
    const transport = new Transport();
    const client = new CyberKindredIpcClient(transport);
    const location = {
      city: "Sydney", region: "New South Wales", country: "Australia", countryCode: "AU",
      latitude: -33.8688, longitude: 151.2093, timezone: "Australia/Sydney",
    };
    transport.response = {
      requestId: REQUEST_ID,
      candidates: [{ candidateId: CANDIDATE_ID, ...location }],
      expiresAt: "2026-09-03T03:10:00.000Z",
    };
    await client.searchWeatherLocations({ clientRequestId: REQUEST_ID, query: "Sydney", limit: 8 });
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_search_weather_locations",
      args: { request: { clientRequestId: REQUEST_ID, query: "Sydney", limit: 8 } },
    });

    transport.response = { requestId: REQUEST_ID, location, revision: 3 };
    await client.selectWeatherLocation({
      clientRequestId: REQUEST_ID, candidateId: CANDIDATE_ID, expectedRevision: 2,
    });
    expect(transport.calls.at(-1)).toEqual({
      command: "api_v1_select_weather_location",
      args: { request: {
        clientRequestId: REQUEST_ID, candidateId: CANDIDATE_ID, expectedRevision: 2,
      } },
    });

    transport.response = {
      requestId: REQUEST_ID,
      candidates: [{ candidateId: CANDIDATE_ID, ...location, latitude: Number.NaN }],
      expiresAt: "2026-09-03T03:10:00.000Z",
    };
    await expect(client.searchWeatherLocations({
      clientRequestId: REQUEST_ID, query: "Sydney", limit: 8,
    })).rejects.toBeInstanceOf(IpcInvocationError);
  });
});
