import { useState, useEffect, useRef } from 'react';
import { Input } from './components/ui/input';
import { Button } from './components/ui/button';

function App() {
	const _handleGenerateRandomDisplayName = (): string => {
		const adjectives = [
			'Swift',
			'Silent',
			'Mighty',
			'Brave',
			'Lucky',
			'Clever',
			'Shiny',
			'Fierce',
			'Gentle',
			'Epic',
		];
		const nouns = [
			'Tiger',
			'Eagle',
			'Wolf',
			'Panda',
			'Phoenix',
			'Falcon',
			'Shark',
			'Bear',
			'Dragon',
			'Hawk',
		];

		const adjective = adjectives[Math.floor(Math.random() * adjectives.length)];
		const noun = nouns[Math.floor(Math.random() * nouns.length)];
		const number = Math.floor(Math.random() * 1000);

		return `${adjective}${noun}${number}`;
	};

	const [displayName, setDisplayName] = useState<string>('');
	const [room, setRoom] = useState('');
	const [translateTo, setTranslateTo] = useState('en');
	const [transcribeTo, setTranscribeTo] = useState('en');
	const [ws, setWs] = useState<WebSocket | null>(null);
	const [wsConnected, setWsConnected] = useState<boolean>(false);
	const mediaStreamRef = useRef<MediaStream | null>(null);

	useEffect(() => {
		setDisplayName(_handleGenerateRandomDisplayName());
	}, []);

	const connectToRoom = async () => {
		if (!room.trim()) {
			alert('Please enter a room name.');
			return;
		}

		// Prepare the mic
		const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
		mediaStreamRef.current = stream;

		const params = new URLSearchParams({
			name: displayName,
			translate_to: translateTo,
			transcribe_to: transcribeTo,
		});

		const socket = new WebSocket(
			`ws://127.0.0.1:12345/${encodeURIComponent(room)}?${params.toString()}`
		);

		socket.binaryType = 'arraybuffer'; // ensure binary is supported

		socket.onopen = () => {
			console.log(`Connected to room: ${room} as ${displayName}`);

			// 🔊 Web Audio API setup for detecting if talking
			const audioContext = new AudioContext();
			const source = audioContext.createMediaStreamSource(stream);
			const analyser = audioContext.createAnalyser();
			analyser.fftSize = 512; // smaller size = faster response
			source.connect(analyser);

			const dataArray = new Uint8Array(analyser.fftSize);
			const threshold = 10; // tweak this sensitivity

			let isTalking = false;

			// 🎤 MediaRecorder for audio chunks
			const recorder = new MediaRecorder(stream, {
				mimeType: 'audio/webm;codecs=opus',
			});

			recorder.addEventListener('dataavailable', ({ data }) => {
				if (data.size > 0 && socket.readyState === WebSocket.OPEN) {
					// check if talking before sending
					analyser.getByteTimeDomainData(dataArray);
					let sum = 0;
					for (const element of dataArray) {
						const v = element - 128;
						sum += v * v;
					}
					const rms = Math.sqrt(sum / dataArray.length);

					if (rms > threshold) {
						if (!isTalking) {
							console.log('🎤 Talking started');
							isTalking = true;
						}
						data.arrayBuffer().then((buffer) => {
							socket.send(buffer);
						});
					} else if (isTalking) {
						console.log('🤫 Idle (no speech)');
						isTalking = false;

						// silence → don’t send
					} else {
						console.log('🤫 not talking');
					}
				}
			});

			// request chunks every 500ms
			recorder.start(500);

			(mediaStreamRef.current as any).recorder = recorder;
		};

		socket.onmessage = (event) => {
			console.log('Message from server:', event.data);
		};

		socket.onclose = () => {
			console.log(`Disconnected from room: ${room}`);
			setWsConnected(false);
			setWs(null);
		};

		socket.onerror = (err) => {
			console.error('WebSocket error:', err);
		};

		setWs(socket);
	};

	const disconnectFromRoom = () => {
		if (ws) {
			// stop recorder if running
			if ((mediaStreamRef.current as any)?.recorder) {
				(mediaStreamRef.current as any).recorder.stop();
			}
			// stop mic input
			mediaStreamRef.current?.getTracks().forEach((track) => track.stop());

			ws.close();
			setWs(null);
			setWsConnected(false);
		}
	};

	return (
		<div className='max-w-2xl mx-auto py-10'>
			<div className='flex w-full justify-center gap-2'>
				<Input
					value={displayName}
					onChange={(e) => setDisplayName(e.target.value)}
					placeholder='Enter your display name...'
					className='w-64'
				/>
				<Input
					value={room}
					onChange={(e) => setRoom(e.target.value)}
					placeholder='Enter room name...'
					className='w-64'
				/>
				<Input
					value={translateTo}
					onChange={(e) => setTranslateTo(e.target.value)}
					placeholder='Translate to...'
					className='w-32'
				/>
				<Input
					value={transcribeTo}
					onChange={(e) => setTranscribeTo(e.target.value)}
					placeholder='Transcribe to...'
					className='w-32'
				/>
				<Button onClick={connectToRoom} disabled={wsConnected}>
					Connect
				</Button>
				<Button
					variant='destructive'
					onClick={disconnectFromRoom}
					disabled={!wsConnected}
				>
					Disconnect
				</Button>
			</div>
		</div>
	);
}

export default App;
