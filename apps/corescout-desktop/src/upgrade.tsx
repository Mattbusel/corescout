/**
 * What this computer has worked out, on the Settings screen.
 *
 * # What this used to be
 *
 * An upgrade screen: a ledger of what the machine had learned, then prices,
 * then a list of what a paid tier would keep doing, then a box for a licence
 * key. CoreScout is now bought once in the Microsoft Store, so there is
 * nothing to sell to somebody already running it and none of that survives.
 *
 * The ledger did. It was always the better half. It is the one thing about
 * this product that only this installation can show, and it is worth showing
 * to somebody who owns it as much as it ever was to somebody deciding.
 *
 * # What it never does
 *
 * Interrupt. There is no modal on launch, no dialogue over the Home screen and
 * no nag on a timer. A trial in its last two days puts one calm line at the top
 * of Settings. That is all, and a purchase puts nothing anywhere: somebody who
 * paid does not need reminding that they paid.
 */

import { type WorkedOut } from "./api";
import { Loading, Problem, useCall } from "./components";

/** What this computer has worked out, shown inside Settings. */
export function Plan() {
  const worked = useCall<WorkedOut>("worked_out", {}, 30_000);

  if (worked.error) return <Problem message={worked.error} retry={worked.reload} />;
  if (!worked.data) return <Loading lines={4} />;
  const data = worked.data;

  return (
    <>
      <div className="section">
        <h2>{data.headline}</h2>
      </div>

      {data.evidence.length > 0 ? (
        <div className="section">
          <div className="eyebrow">What this computer worked out</div>
          <div className="ledger">
            {data.evidence.map((item) => (
              <div className="ledger-row" key={item.label}>
                <span className="ledger-value">{item.value}</span>
                <span>{item.label}</span>
              </div>
            ))}
          </div>
        </div>
      ) : (
        <div className="section">
          <p className="lede">
            CoreScout has not worked anything out yet. Leave it running and
            connect an AI, and this fills in on its own.
          </p>
        </div>
      )}
    </>
  );
}

/**
 * One calm line, at the top of Settings, and only when there is a reason.
 *
 * The reason is a trial about to end or a licence that is no longer valid.
 * Everything else, including owning CoreScout, is silence.
 */
export function LicenceNotice({ licence }: { licence: WorkedOut | null | undefined }) {
  if (!licence?.worth_mentioning) return null;
  return (
    <div className="banner">
      <span>{licence.headline}</span>
    </div>
  );
}
