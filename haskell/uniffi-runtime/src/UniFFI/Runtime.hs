module UniFFI.Runtime
  ( RustBuffer (..)
  , ForeignBytes (..)
  , RustCallStatus (..)
  , UniFFIException (..)
  , RustBufferFromBytes
  , RustBufferFree
  , RustObject
  , newRustObject
  , withRustObject
  , finalizeRustObject
  , emptyRustBuffer
  , successRustCallStatus
  , withRustCallStatus
  , lowerRustBuffer
  , consumeRustBuffer
  , checkRustCallStatus
  , checkRustCallStatusWithError
  , encodeUtf8
  , decodeUtf8
  , Encoder
  , runEncoder
  , writeWord8
  , writeWord16
  , writeWord32
  , writeWord64
  , writeInt8
  , writeInt16
  , writeInt32
  , writeInt64
  , writeFloat
  , writeDouble
  , writeBool
  , writeText
  , writeBytes
  , writeMaybe
  , writeList
  , Decoder
  , runDecoder
  , readWord8
  , readWord16
  , readWord32
  , readWord64
  , readInt8
  , readInt16
  , readInt32
  , readInt64
  , readFloat
  , readDouble
  , readBool
  , readText
  , readBytes
  , readMaybe
  , readList
  , serializeBytes
  , deserializeBytes
  ) where

import Control.Concurrent.MVar (MVar, modifyMVar_, newMVar, withMVar)
import Control.Exception (Exception, SomeException, catch, finally, throw, throwIO)
import Control.Monad (replicateM, unless, void, when)
import Data.Bits (Bits, (.|.), shiftL, shiftR)
import Data.ByteString (ByteString)
import qualified Data.ByteString as ByteString
import qualified Data.ByteString.Builder as Builder
import qualified Data.ByteString.Lazy as LazyByteString
import Data.Int (Int8, Int16, Int32, Int64)
import Data.Text (Text)
import qualified Data.Text as Text
import qualified Data.Text.Encoding as TextEncoding
import Data.Text.Encoding.Error (UnicodeException)
import Data.Word (Word8, Word16, Word32, Word64)
import qualified Foreign.Concurrent as ForeignConcurrent
import Foreign.ForeignPtr (ForeignPtr, finalizeForeignPtr, withForeignPtr)
import Foreign.Marshal.Alloc (alloca, free, mallocBytes)
import Foreign.Marshal.Utils (with)
import Foreign.Ptr (Ptr, castPtr, nullPtr)
import Foreign.Storable (Storable (..), peekByteOff, pokeByteOff)
import GHC.Float (castWord32ToFloat, castWord64ToDouble)
import Prelude hiding (readList)


data RustBuffer = RustBuffer
  { rustBufferCapacity :: Word64
  , rustBufferLen :: Word64
  , rustBufferData :: Ptr Word8
  }
  deriving (Eq, Show)

instance Storable RustBuffer where
  sizeOf _ = rustBufferSize
  alignment _ = rustBufferAlignment
  peek ptr =
    RustBuffer
      <$> peekByteOff ptr rustBufferCapacityOffset
      <*> peekByteOff ptr rustBufferLenOffset
      <*> peekByteOff ptr rustBufferDataOffset
  poke ptr buffer = do
    pokeByteOff ptr rustBufferCapacityOffset (rustBufferCapacity buffer)
    pokeByteOff ptr rustBufferLenOffset (rustBufferLen buffer)
    pokeByteOff ptr rustBufferDataOffset (rustBufferData buffer)


data ForeignBytes = ForeignBytes
  { foreignBytesLen :: Int32
  , foreignBytesData :: Ptr Word8
  }
  deriving (Eq, Show)

instance Storable ForeignBytes where
  sizeOf _ = foreignBytesSize
  alignment _ = foreignBytesAlignment
  peek ptr =
    ForeignBytes
      <$> peekByteOff ptr foreignBytesLenOffset
      <*> peekByteOff ptr foreignBytesDataOffset
  poke ptr bytes = do
    pokeByteOff ptr foreignBytesLenOffset (foreignBytesLen bytes)
    pokeByteOff ptr foreignBytesDataOffset (foreignBytesData bytes)


data RustCallStatus = RustCallStatus
  { rustCallStatusCode :: Int8
  , rustCallStatusError :: RustBuffer
  }
  deriving (Eq, Show)

instance Storable RustCallStatus where
  sizeOf _ = rustCallStatusSize
  alignment _ = rustCallStatusAlignment
  peek ptr =
    RustCallStatus
      <$> peekByteOff ptr rustCallStatusCodeOffset
      <*> peekByteOff ptr rustCallStatusErrorOffset
  poke ptr status = do
    pokeByteOff ptr rustCallStatusCodeOffset (rustCallStatusCode status)
    pokeByteOff ptr rustCallStatusErrorOffset (rustCallStatusError status)


newtype UniFFIException = UniFFIException Text
  deriving (Eq, Show)

instance Exception UniFFIException

type RustBufferFromBytes = Ptr ForeignBytes -> Ptr RustBuffer -> Ptr RustCallStatus -> IO ()

type RustBufferFree = Ptr RustBuffer -> Ptr RustCallStatus -> IO ()

data RustObject = RustObject (MVar (Maybe Word64)) (ForeignPtr ())

newRustObject :: Word64 -> (Word64 -> IO ()) -> IO RustObject
newRustObject handle release
  | handle == 0 = throwUniFFIException "UniFFI returned a null object handle"
  | otherwise = do
      state <- newMVar (Just handle)
      pointer <- mallocBytes 1
      foreignPointer <-
        ForeignConcurrent.newForeignPtr pointer $
          releaseRustObject state release `finally` free pointer
      pure (RustObject state foreignPointer)

withRustObject :: RustObject -> (Word64 -> IO a) -> IO a
withRustObject (RustObject state foreignPointer) action =
  withForeignPtr foreignPointer $ \_ ->
    withMVar state $ \maybeHandle ->
      case maybeHandle of
        Just handle -> action handle
        Nothing -> throwUniFFIException "UniFFI object has already been closed"

finalizeRustObject :: RustObject -> IO ()
finalizeRustObject (RustObject _ foreignPointer) = finalizeForeignPtr foreignPointer

releaseRustObject :: MVar (Maybe Word64) -> (Word64 -> IO ()) -> IO ()
releaseRustObject state release =
  modifyMVar_ state $ \maybeHandle ->
    case maybeHandle of
      Nothing -> pure Nothing
      Just handle -> do
        release handle `catch` ignoreFinalizerException
        pure Nothing

ignoreFinalizerException :: SomeException -> IO ()
ignoreFinalizerException _ = pure ()

emptyRustBuffer :: RustBuffer
emptyRustBuffer = RustBuffer 0 0 nullPtr

successRustCallStatus :: RustCallStatus
successRustCallStatus = RustCallStatus callSuccess emptyRustBuffer

withRustCallStatus :: (Ptr RustCallStatus -> IO a) -> IO (a, RustCallStatus)
withRustCallStatus action =
  alloca $ \statusPtr -> do
    poke statusPtr successRustCallStatus
    result <- action statusPtr
    status <- peek statusPtr
    pure (result, status)

lowerRustBuffer :: RustBufferFromBytes -> RustBufferFree -> ByteString -> IO RustBuffer
lowerRustBuffer fromBytes freeBuffer bytes =
  ByteString.useAsCStringLen bytes $ \(bytesPtr, byteLength) -> do
    when (byteLength > fromIntegral (maxBound :: Int32)) $
      throwUniFFIException "ByteString is too large for ForeignBytes"
    alloca $ \bufferPtr -> do
      poke bufferPtr emptyRustBuffer
      (_, status) <-
        withRustCallStatus $ \statusPtr ->
          with (ForeignBytes (fromIntegral byteLength) (castPtr bytesPtr)) $ \foreignBytesPtr ->
            fromBytes foreignBytesPtr bufferPtr statusPtr
      checkRustCallStatus freeBuffer status
      peek bufferPtr

consumeRustBuffer :: RustBufferFree -> RustBuffer -> IO ByteString
consumeRustBuffer freeBuffer buffer =
  copyRustBuffer buffer `finally` releaseRustBuffer freeBuffer buffer

checkRustCallStatus :: RustBufferFree -> RustCallStatus -> IO ()
checkRustCallStatus freeBuffer status =
  case rustCallStatusCode status of
    code
      | code == callSuccess -> pure ()
      | code == callError -> do
          void (consumeRustBuffer freeBuffer (rustCallStatusError status))
          throwUniFFIException "Rust call returned an expected error"
      | code == callUnexpectedError -> do
          bytes <- consumeRustBuffer freeBuffer (rustCallStatusError status)
          case decodeUtf8 bytes of
            Left decodingError ->
              throwUniFFIException ("Unexpected Rust error was not valid UTF-8: " ++ show decodingError)
            Right message -> throwIO (UniFFIException message)
      | code == callCancelled ->
          throwUniFFIException "Rust call was cancelled"
      | otherwise ->
          throwUniFFIException ("Unknown Rust call status code: " ++ show code)

checkRustCallStatusWithError :: RustBufferFree -> Decoder error -> RustCallStatus -> IO (Maybe error)
checkRustCallStatusWithError freeBuffer decodeError status
  | rustCallStatusCode status == callSuccess = pure Nothing
  | rustCallStatusCode status == callError = do
      bytes <- consumeRustBuffer freeBuffer (rustCallStatusError status)
      Just <$> runDecoder decodeError bytes
  | otherwise = do
      checkRustCallStatus freeBuffer status
      pure Nothing

encodeUtf8 :: Text -> ByteString
encodeUtf8 = TextEncoding.encodeUtf8

decodeUtf8 :: ByteString -> Either UnicodeException Text
decodeUtf8 = TextEncoding.decodeUtf8'

newtype Encoder = Encoder Builder.Builder

instance Semigroup Encoder where
  Encoder first <> Encoder second = Encoder (first <> second)

instance Monoid Encoder where
  mempty = Encoder mempty

runEncoder :: Encoder -> ByteString
runEncoder (Encoder builder) = LazyByteString.toStrict (Builder.toLazyByteString builder)

writeWord8 :: Word8 -> Encoder
writeWord8 = Encoder . Builder.word8

writeWord16 :: Word16 -> Encoder
writeWord16 = Encoder . Builder.word16BE

writeWord32 :: Word32 -> Encoder
writeWord32 = Encoder . Builder.word32BE

writeWord64 :: Word64 -> Encoder
writeWord64 = Encoder . Builder.word64BE

writeInt8 :: Int8 -> Encoder
writeInt8 = Encoder . Builder.int8

writeInt16 :: Int16 -> Encoder
writeInt16 = Encoder . Builder.int16BE

writeInt32 :: Int32 -> Encoder
writeInt32 = Encoder . Builder.int32BE

writeInt64 :: Int64 -> Encoder
writeInt64 = Encoder . Builder.int64BE

writeFloat :: Float -> Encoder
writeFloat = Encoder . Builder.floatBE

writeDouble :: Double -> Encoder
writeDouble = Encoder . Builder.doubleBE

writeBool :: Bool -> Encoder
writeBool False = writeWord8 0
writeBool True = writeWord8 1

writeText :: Text -> Encoder
writeText = writeBytes . encodeUtf8

writeBytes :: ByteString -> Encoder
writeBytes bytes =
  writeLength "byte string" (ByteString.length bytes)
    <> Encoder (Builder.byteString bytes)

writeMaybe :: (a -> Encoder) -> Maybe a -> Encoder
writeMaybe _ Nothing = writeWord8 0
writeMaybe writeValue (Just value) = writeWord8 1 <> writeValue value

writeList :: (a -> Encoder) -> [a] -> Encoder
writeList writeValue values =
  writeLength "list" (length values) <> foldMap writeValue values

newtype Decoder a = Decoder (ByteString -> Either String (a, ByteString))

instance Functor Decoder where
  fmap transform (Decoder decode) =
    Decoder $ \input -> do
      (value, remaining) <- decode input
      pure (transform value, remaining)

instance Applicative Decoder where
  pure value = Decoder $ \input -> Right (value, input)
  Decoder decodeFunction <*> Decoder decodeValue =
    Decoder $ \input -> do
      (function, afterFunction) <- decodeFunction input
      (value, remaining) <- decodeValue afterFunction
      pure (function value, remaining)

instance Monad Decoder where
  Decoder decode >>= next =
    Decoder $ \input -> do
      (value, remaining) <- decode input
      case next value of
        Decoder decodeNext -> decodeNext remaining

instance MonadFail Decoder where
  fail message = Decoder $ \_ -> Left message

runDecoder :: Decoder a -> ByteString -> IO a
runDecoder (Decoder decode) bytes =
  case decode bytes of
    Left message -> throwUniFFIException message
    Right (value, remaining)
      | ByteString.null remaining -> pure value
      | otherwise ->
          throwUniFFIException
            ( "UniFFI decoder left "
                ++ show (ByteString.length remaining)
                ++ " trailing bytes"
            )

readWord8 :: Decoder Word8
readWord8 = do
  bytes <- takeDecoderBytes "Word8" 1
  pure (ByteString.index bytes 0)

readWord16 :: Decoder Word16
readWord16 = decodeBigEndian "Word16" 2

readWord32 :: Decoder Word32
readWord32 = decodeBigEndian "Word32" 4

readWord64 :: Decoder Word64
readWord64 = decodeBigEndian "Word64" 8

readInt8 :: Decoder Int8
readInt8 = fromIntegral <$> readWord8

readInt16 :: Decoder Int16
readInt16 = fromIntegral <$> readWord16

readInt32 :: Decoder Int32
readInt32 = fromIntegral <$> readWord32

readInt64 :: Decoder Int64
readInt64 = fromIntegral <$> readWord64

readFloat :: Decoder Float
readFloat = castWord32ToFloat <$> readWord32

readDouble :: Decoder Double
readDouble = castWord64ToDouble <$> readWord64

readBool :: Decoder Bool
readBool = do
  tag <- readWord8
  case tag of
    0 -> pure False
    1 -> pure True
    _ -> fail ("Invalid Bool tag: " ++ show tag)

readText :: Decoder Text
readText = do
  bytes <- readBytes
  case decodeUtf8 bytes of
    Left decodingError -> fail ("Invalid UTF-8: " ++ show decodingError)
    Right value -> pure value

readBytes :: Decoder ByteString
readBytes = do
  byteLength <- readInt32
  if byteLength < 0
    then fail ("Negative byte string length: " ++ show byteLength)
    else takeDecoderBytes "byte string payload" (fromIntegral byteLength)

readMaybe :: Decoder a -> Decoder (Maybe a)
readMaybe readValue = do
  tag <- readWord8
  case tag of
    0 -> pure Nothing
    1 -> Just <$> readValue
    _ -> fail ("Invalid Maybe tag: " ++ show tag)

readList :: Decoder a -> Decoder [a]
readList readValue = do
  count <- readInt32
  if count < 0
    then fail ("Negative list count: " ++ show count)
    else replicateM (fromIntegral count) readValue

writeLength :: String -> Int -> Encoder
writeLength description value
  | value > fromIntegral (maxBound :: Int32) =
      throwEncoderException ("UniFFI " ++ description ++ " length exceeds the Int32 range")
  | otherwise = writeInt32 (fromIntegral value)

throwEncoderException :: String -> a
throwEncoderException = throw . UniFFIException . Text.pack

takeDecoderBytes :: String -> Int -> Decoder ByteString
takeDecoderBytes description count =
  Decoder $ \input ->
    if ByteString.length input < count
      then
        Left
          ( "Truncated "
              ++ description
              ++ ": needed "
              ++ show count
              ++ " bytes, found "
              ++ show (ByteString.length input)
          )
      else Right (ByteString.splitAt count input)

decodeBigEndian :: (Bits a, Num a) => String -> Int -> Decoder a
decodeBigEndian description byteCount = do
  bytes <- takeDecoderBytes description byteCount
  pure (ByteString.foldl' appendByte 0 bytes)
  where
    appendByte value byte = (value `shiftL` 8) .|. fromIntegral byte

serializeBytes :: ByteString -> IO ByteString
serializeBytes bytes = do
  let byteLength = ByteString.length bytes
  when (byteLength > fromIntegral (maxBound :: Int32)) $
    throwUniFFIException "ByteString is too large for UniFFI serialization"
  let lengthWord = fromIntegral byteLength :: Word32
      prefix =
        ByteString.pack
          [ fromIntegral (lengthWord `shiftR` 24)
          , fromIntegral (lengthWord `shiftR` 16)
          , fromIntegral (lengthWord `shiftR` 8)
          , fromIntegral lengthWord
          ]
  pure (prefix <> bytes)

deserializeBytes :: ByteString -> IO ByteString
deserializeBytes bytes
  | ByteString.length bytes < 4 =
      throwUniFFIException "UniFFI bytes value is missing its length prefix"
  | otherwise = do
      let byte index = fromIntegral (ByteString.index bytes index) :: Word32
          declaredLength =
            (byte 0 `shiftL` 24)
              .|. (byte 1 `shiftL` 16)
              .|. (byte 2 `shiftL` 8)
              .|. byte 3
          payload = ByteString.drop 4 bytes
      when (declaredLength > fromIntegral (maxBound :: Int)) $
        throwUniFFIException "UniFFI bytes length exceeds the host Int range"
      unless (ByteString.length payload == fromIntegral declaredLength) $
        throwUniFFIException "UniFFI bytes length does not match its payload"
      pure payload

copyRustBuffer :: RustBuffer -> IO ByteString
copyRustBuffer buffer = do
  let capacity = rustBufferCapacity buffer
      len = rustBufferLen buffer
      dataPtr = rustBufferData buffer
  when (len > capacity) $
    throwUniFFIException "RustBuffer length exceeds capacity"
  when (len > fromIntegral (maxBound :: Int)) $
    throwUniFFIException "RustBuffer length exceeds the host Int range"
  when (dataPtr == nullPtr) $
    unless (capacity == 0 && len == 0) $
      throwUniFFIException "Null RustBuffer data had a non-zero length or capacity"
  if len == 0
    then pure ByteString.empty
    else ByteString.packCStringLen (castPtr dataPtr, fromIntegral len)

releaseRustBuffer :: RustBufferFree -> RustBuffer -> IO ()
releaseRustBuffer freeBuffer buffer = do
  (_, status) <-
    withRustCallStatus $ \statusPtr ->
      with buffer $ \bufferPtr ->
        freeBuffer bufferPtr statusPtr
  checkRustCallStatus freeBuffer status

throwUniFFIException :: String -> IO a
throwUniFFIException = throwIO . UniFFIException . Text.pack

callSuccess :: Int8
callSuccess = 0

callError :: Int8
callError = 1

callUnexpectedError :: Int8
callUnexpectedError = 2

callCancelled :: Int8
callCancelled = 3

rustBufferCapacityOffset :: Int
rustBufferCapacityOffset = 0

rustBufferLenOffset :: Int
rustBufferLenOffset =
  alignUp
    (rustBufferCapacityOffset + sizeOf (undefined :: Word64))
    (alignment (undefined :: Word64))

rustBufferDataOffset :: Int
rustBufferDataOffset =
  alignUp
    (rustBufferLenOffset + sizeOf (undefined :: Word64))
    (alignment (undefined :: Ptr Word8))

rustBufferAlignment :: Int
rustBufferAlignment =
  max
    (alignment (undefined :: Word64))
    (alignment (undefined :: Ptr Word8))

rustBufferSize :: Int
rustBufferSize =
  alignUp
    (rustBufferDataOffset + sizeOf (undefined :: Ptr Word8))
    rustBufferAlignment

foreignBytesLenOffset :: Int
foreignBytesLenOffset = 0

foreignBytesDataOffset :: Int
foreignBytesDataOffset =
  alignUp
    (foreignBytesLenOffset + sizeOf (undefined :: Int32))
    (alignment (undefined :: Ptr Word8))

foreignBytesAlignment :: Int
foreignBytesAlignment =
  max
    (alignment (undefined :: Int32))
    (alignment (undefined :: Ptr Word8))

foreignBytesSize :: Int
foreignBytesSize =
  alignUp
    (foreignBytesDataOffset + sizeOf (undefined :: Ptr Word8))
    foreignBytesAlignment

rustCallStatusCodeOffset :: Int
rustCallStatusCodeOffset = 0

rustCallStatusErrorOffset :: Int
rustCallStatusErrorOffset =
  alignUp
    (rustCallStatusCodeOffset + sizeOf (undefined :: Int8))
    (alignment (undefined :: RustBuffer))

rustCallStatusAlignment :: Int
rustCallStatusAlignment =
  max
    (alignment (undefined :: Int8))
    (alignment (undefined :: RustBuffer))

rustCallStatusSize :: Int
rustCallStatusSize =
  alignUp
    (rustCallStatusErrorOffset + sizeOf (undefined :: RustBuffer))
    rustCallStatusAlignment

alignUp :: Int -> Int -> Int
alignUp offset boundary = ((offset + boundary - 1) `div` boundary) * boundary
