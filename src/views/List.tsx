import { Spinner } from '@/components/ui/spinner';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useRef, useState } from 'react';

type StreamerUpdate = {
  broadcaster_id: string;
  broadcaster_name: string;
  title: string;
  category: string;
  isLive: boolean;
};

interface IListProps {
  loading: boolean;
}

export const List = ({ loading }: IListProps) => {
  const [updates, setUpdates] = useState<StreamerUpdate[]>([]);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const startingRef = useRef(false);

  useEffect(() => {
    (async () => {
      if (startingRef.current) return;
      startingRef.current = true;

      if (unlistenRef.current) {
        startingRef.current = false;
        return;
      }

      try {
        const unlisten = await listen('streamer:update', (event) => {
          const payload = event.payload as StreamerUpdate;
          setUpdates((prev) => {
            const out = [...prev, payload];
            if (out.length > 500) out.shift();
            return out;
          });
        });

        unlistenRef.current = unlisten;
      } catch (err) {
        console.error('failed to start Tauri listener', err);
      } finally {
        startingRef.current = false;
      }
    })();

    return () => {
      const unlisten = unlistenRef.current;
      if (unlisten) {
        unlistenRef.current = null;
      }
    };
  }, []);

  return (
    <>
      {loading ? (
        <div className='flex flex-row justify-center'>
          <Spinner variant="infinite" size={64} />
        </div>
      ) : (
        <>
          {updates.length === 0 ? (
            <p>List is empty.</p>
          ) : (
            <ul>
              {updates.map((u, i) => (
                <li key={i}>{JSON.stringify(u)}</li>
              ))}
            </ul>
          )}
        </>
      )}
    </>
  );
};
